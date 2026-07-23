import { MOCK, mockInvoke, mockSubscribe } from "./mockPreview";

/**
 * The one place the UI decides HOW it reaches `genaryx-core`.
 *
 * The console is a web app: these React components talk HTTP to a
 * `genaryx-web` backend co-located with the customer's stack. No panel should
 * know that, so every plane module (`lib/money.ts`, `lib/policy.ts`, ...)
 * calls through here instead of touching `fetch` directly. The only other
 * transport is the mock preview (`.env.mock`), which routes every command to
 * `src/lib/mockPreview.ts` so the UI can be demoed with no backend at all.
 * (A Tauri IPC branch lived here when the product also shipped native desktop
 * shells; those left with the web-only pivot.)
 *
 * The contract survives from that era on purpose: resolve with the command's
 * Ok value, reject with its Err value as the SAME structured object each
 * plane's own error-normaliser (`toMoneyError`, `toPolicyError`, ...) already
 * expects. `genaryx-web` mirrors the command layer one for one: same command
 * names, same args, byte-identical serde DTOs, and it returns a command's Err
 * as the JSON body on a non-2xx. So nothing downstream changes.
 */

/** Where the build sends commands. Set at build time (`.env.web`:
 * `VITE_GENARYX_API=/api`); undefined in a bare `vite preview` and in the
 * mock build, which have no backend at all. */
const WEB_API_BASE: string | undefined = import.meta.env.VITE_GENARYX_API;

/** True when there is a real backend to call (a configured web API) or a mock
 * one. False in a bare preview, where callers fall back to mock data or a
 * "no environment" state exactly as they did before this seam existed. */
export function hasBackend(): boolean {
  return Boolean(WEB_API_BASE) || MOCK;
}

/** True when this console is talking to a `genaryx-web` on the customer's own
 * box, so it needs a signed-in session before it can read anything. False in
 * the mock preview, which has no login gate. */
export function isWebShell(): boolean {
  return Boolean(WEB_API_BASE);
}

/** Base URL of the web backend, for the few callers that need more than
 * `invokeBackend` (the sign-in gate, the live-event stream). */
export function webApiBase(): string {
  return WEB_API_BASE ?? "";
}

/** Console roles, as `genaryx-web`'s command chokepoint names them
 * (docs/CONSOLE-IDP.md "Role gating", `crates/web/src/roles.rs`'s `Role`).
 * Shared by `lib/session.ts`'s `Session.role` and {@link requiredRoleFromCommandError}
 * below, so "who am I" and "what did this refuse" speak the same type. */
export type ConsoleRole = "viewer" | "approver" | "admin";

/**
 * True when `err` is `genaryx-web`'s role-gate refusal - a `403 {"error":
 * "role <x> required"}` the command chokepoint returns BEFORE a command ever
 * reaches its own domain handler (docs/CONSOLE-IDP.md "Role gating"). This is
 * never a `MoneyError`/`PolicyError`/...-shaped rejection (those are always
 * tagged with their own `kind`), so it needs its own recognizer - kept here,
 * the one place that already speaks the raw wire shape, so every plane's
 * normalizer (`toMoneyError`, `toPolicyError`) can fold the SAME check into
 * its own typed `role_required` variant instead of re-deriving the message
 * format per plane.
 */
export function requiredRoleFromCommandError(err: unknown): ConsoleRole | null {
  if (!err || typeof err !== "object" || "kind" in err) return null;
  const message = (err as { error?: unknown }).error;
  if (typeof message !== "string") return null;
  const m = /^role (viewer|approver|admin) required$/.exec(message);
  return (m?.[1] as ConsoleRole | undefined) ?? null;
}

/**
 * Invoke a core command over whichever transport is live (HTTP or mock).
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
  if (MOCK) {
    return mockInvoke<T>(command, args);
  }
  if (!WEB_API_BASE) {
    throw new Error(
      `no backend: cannot invoke "${command}" without VITE_GENARYX_API`,
    );
  }
  const resp = await fetch(`${WEB_API_BASE}/command/${command}`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(args ?? {}),
    // The console authenticates with a session cookie set at login.
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
    // genaryx-web returns the command's structured error as the body on a
    // non-2xx, so the plane's own normaliser unwraps it exactly as it always
    // has - reject with the Err value itself, never a wrapper.
    throw body;
  }
  return body as T;
}

/**
 * The live bus, as Server-Sent Events from `genaryx-web`.
 *
 * Callers hand in the event name they subscribe to and get back an
 * unsubscribe function, so a panel that redraws on a new bus event needs no
 * idea what carries it. This is also how the Remote panel's live SSH tail
 * reaches the browser: its two event names (`remote:tail-line`/
 * `remote:tail-ended`) ride the SAME `genaryx-web` SSE stream, each under its
 * own named SSE event rather than the bus's `UiEvent` shape, which a raw
 * remote log line does not fit (see `crates/web/src/ctx.rs`'s
 * `RemoteTailEvent` doc comment for the backend side of that split).
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
  if (MOCK) {
    return mockSubscribe<T>(event, onEvent);
  }
  if (!WEB_API_BASE) {
    // No backend at all (a bare preview): there is nothing to stream, and a
    // caller that also fell back to mock data must not be left waiting.
    return () => {};
  }
  return subscribeSse(sseEventNameFor(event), onEvent as (payload: unknown) => void);
}

/**
 * The SSE `event:` name a given bus event name arrives under on
 * `genaryx-web`'s `/events` stream.
 *
 * The live bus feed is the one legacy exception: its SSE frames are named
 * `"bus"`, not `"bus:event"`, from back when this stream only ever carried
 * one shape - kept as its own literal mapping (rather than renamed to match)
 * so every existing subscriber (`BusExplorer`, `DecisionStream`, ...) keeps
 * working unchanged. Every other event name arrives under its own literal
 * name - which is what `crates/web/src/main.rs`'s `events` handler emits a
 * remote tail's `remote:tail-line`/`remote:tail-ended` frames under.
 */
function sseEventNameFor(event: string): string {
  return event === "bus:event" ? "bus" : event;
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
