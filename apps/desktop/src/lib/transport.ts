import { invoke, isTauri } from "@tauri-apps/api/core";

/**
 * The one place the UI decides HOW it reaches `genaryx-core`.
 *
 * The same React components run two ways: as the desktop shell over Tauri IPC,
 * or as the web app over HTTP to a `genaryx-web` backend co-located with the
 * customer's stack. Neither panel should know which, so every plane module
 * (`lib/money.ts`, `lib/policy.ts`, ...) calls through here instead of
 * importing `@tauri-apps/api/core` directly.
 *
 * The contract is deliberately identical to Tauri's `invoke`: resolve with the
 * command's Ok value, reject with its Err value as the SAME structured object
 * each plane's own error-normaliser (`toMoneyError`, `toPolicyError`, ...)
 * already expects. The HTTP backend mirrors the Tauri commands one for one:
 * same command names, same args, byte-identical serde DTOs, and it returns a
 * command's Err as the JSON body on a non-2xx. So nothing downstream changes.
 */

/** Where the web build sends commands. Undefined in the desktop build (which
 * uses Tauri) and in a bare `vite preview` (no backend at all). Set at
 * web-build time, e.g. `VITE_GENARYX_API=/api`. */
const WEB_API_BASE: string | undefined = import.meta.env.VITE_GENARYX_API;

/** True when there is a real backend to call: a Tauri runtime, or a configured
 * web API. False in a bare preview, where callers fall back to mock data or a
 * "no environment" state exactly as they did before this seam existed. */
export function hasBackend(): boolean {
  return isTauri() || Boolean(WEB_API_BASE);
}

/** True in the browser build: this console is talking to a `genaryx-web` on
 * the customer's own box, so it needs a signed-in session before it can read
 * anything. The desktop build is already on that box and has no such gate. */
export function isWebShell(): boolean {
  return !isTauri() && Boolean(WEB_API_BASE);
}

/** Base URL of the web backend, for the few callers that need more than
 * `invokeBackend` (the sign-in gate, the live-event stream). */
export function webApiBase(): string {
  return WEB_API_BASE ?? "";
}

/**
 * Invoke a core command over whichever transport is live.
 *
 * Resolves with the command's result; rejects with its structured error value
 * (never wrapped), so each plane's normaliser keeps working unchanged. Throws
 * a plain `Error` only when there is no backend at all, which callers already
 * guard against with {@link hasBackend}.
 */
export async function invokeBackend<T>(
  command: string,
  args?: Record<string, unknown>,
): Promise<T> {
  if (isTauri()) {
    return invoke<T>(command, args);
  }
  if (!WEB_API_BASE) {
    throw new Error(
      `no backend: cannot invoke "${command}" without a Tauri runtime or VITE_GENARYX_API`,
    );
  }
  const resp = await fetch(`${WEB_API_BASE}/command/${command}`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(args ?? {}),
    // The web build authenticates with a session cookie set at login; the
    // desktop path returns above and never reaches here.
    credentials: "include",
  });

  const text = await resp.text();
  let body: unknown = null;
  if (text) {
    try {
      body = JSON.parse(text);
    } catch {
      // A non-JSON body (a proxy error page, say) is not a command Err; keep
      // the raw text so the plane normaliser's string fallback can surface it.
      body = text;
    }
  }

  if (!resp.ok) {
    // Mirror invoke()'s reject-with-the-Err-value: genaryx-web returns the
    // command's structured error as the body on a non-2xx, so the plane's own
    // normaliser unwraps it exactly as it does a Tauri Err.
    throw body;
  }
  return body as T;
}

/**
 * The live bus, whichever shell we are in.
 *
 * The desktop shell gets it as a Tauri event; the web build gets the same
 * payloads as Server-Sent Events from `genaryx-web`. Callers hand in the
 * Tauri event name they already use and get back an unsubscribe function, so
 * a panel that redraws on a new bus event needs no idea which shell it is in.
 * This is also how the Remote panel's live SSH tail reaches the browser: its
 * two Tauri events (`remote:tail-line`/`remote:tail-ended`) ride the SAME
 * `genaryx-web` SSE stream, each under its own named SSE event rather than
 * the bus's `UiEvent` shape, which a raw remote log line does not fit (see
 * `crates/web/src/ctx.rs`'s `RemoteTailEvent` doc comment for the backend
 * side of that split).
 *
 * One `EventSource` is shared by every subscriber and closed when the last
 * one leaves, across every event name: seven-plus panels each opening their
 * own connection would spend seven of the browser's six-per-origin
 * connections doing the same work.
 */
export async function subscribeBackend<T>(
  event: string,
  onEvent: (payload: T) => void,
): Promise<() => void> {
  if (isTauri()) {
    const { listen } = await import("@tauri-apps/api/event");
    return listen<T>(event, (e) => onEvent(e.payload));
  }
  if (!WEB_API_BASE) {
    // No backend at all (a bare preview): there is nothing to stream, and a
    // caller that also fell back to mock data must not be left waiting.
    return () => {};
  }
  return subscribeSse(sseEventNameFor(event), onEvent as (payload: unknown) => void);
}

/**
 * The SSE `event:` name a given Tauri event name arrives under on
 * `genaryx-web`'s `/events` stream.
 *
 * The live bus feed is the one legacy exception: its SSE frames are named
 * `"bus"`, not `"bus:event"`, from back when this stream only ever carried
 * one shape - kept as its own literal mapping (rather than renamed to match)
 * so every existing subscriber (`BusExplorer`, `DecisionStream`, ...) keeps
 * working unchanged. Every other event name arrives under its own literal
 * name, matching the Tauri event name exactly - which is what
 * `crates/web/src/main.rs`'s `events` handler emits a remote tail's
 * `remote:tail-line`/`remote:tail-ended` frames under.
 */
function sseEventNameFor(tauriEvent: string): string {
  return tauriEvent === "bus:event" ? "bus" : tauriEvent;
}

/** Subscribers to each named SSE event, keyed by the SSE event name - all
 * names share the ONE `EventSource` connection (see this module's doc
 * comment), but only the subscribers registered for a given name ever see
 * that name's frames. */
const sseSubscribers = new Map<string, Set<(payload: unknown) => void>>();
let sseSource: EventSource | undefined;

function subscribeSse(sseEvent: string, onEvent: (payload: unknown) => void): () => void {
  if (!sseSource) {
    sseSource = new EventSource(`${WEB_API_BASE}/events`, {
      withCredentials: true,
    });
    // The browser reconnects an EventSource on its own, so an error here is
    // usually a tunnel blip rather than a fatal state. Left to reconnect
    // deliberately; tearing it down would need every panel to resubscribe.
  }

  let subscribers = sseSubscribers.get(sseEvent);
  if (!subscribers) {
    subscribers = new Set();
    sseSubscribers.set(sseEvent, subscribers);
    // One listener per DISTINCT SSE event name, bound once and shared by
    // every subscriber to that name - added only the first time this name is
    // seen, so a `bus` frame is never handed to a `remote:tail-line`
    // subscriber or vice versa.
    sseSource.addEventListener(sseEvent, (e) => {
      let payload: unknown;
      try {
        payload = JSON.parse((e as MessageEvent<string>).data);
      } catch {
        // A frame we cannot parse is a frame we must not hand on as if we
        // could. Drop it; the panels' own reads remain the source of truth.
        return;
      }
      for (const fn of sseSubscribers.get(sseEvent) ?? []) fn(payload);
    });
  }
  subscribers.add(onEvent);

  return () => {
    subscribers.delete(onEvent);
    // Only tear down the shared connection once EVERY name's subscriber set
    // is empty - an empty set for one name is left in place rather than
    // deleted, so a later resubscribe to that same name reuses the listener
    // already bound above instead of double-binding a second one onto the
    // same still-live `EventSource`.
    const allEmpty = [...sseSubscribers.values()].every((set) => set.size === 0);
    if (allEmpty) {
      sseSource?.close();
      sseSource = undefined;
      sseSubscribers.clear();
    }
  };
}
