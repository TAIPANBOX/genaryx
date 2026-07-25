// `MOCK` from `mockPreview.ts` directly, not `lib/transport.ts`: transport.ts
// imports it too but never re-exports it, so this is the one public source
// for it, same as `transport.ts`'s own import.
import { MOCK } from "../lib/mockPreview";

/**
 * Zero-egress guard for the Live Demo sandbox (it-rat.com "Live demo").
 *
 * The demo build already has no backend to call: `transport.ts`'s
 * `invokeBackend`/`subscribeBackend` short-circuit to `mockPreview.ts`
 * before ever touching `fetch`/`EventSource`, and `.env.mock` sets no
 * `VITE_GENARYX_API` at all. This module is the belt-and-suspenders layer
 * on top of that, so even a stray call (a future change, a copy-pasted
 * panel, a bug) can never leave the browser. It monkey-patches the four
 * ways browser JS reaches the network, fetch, XMLHttpRequest, WebSocket,
 * EventSource, into no-ops, and short-circuits `navigator.credentials` so
 * no OS passkey prompt can ever appear either (see the WebAuthn section
 * below for why that path is not actually reachable today, and why it is
 * patched anyway).
 *
 * Imported once, for its side effect only, at the very top of `main.tsx`,
 * before React, before `App`, before anything else has a chance to run and
 * capture an unpatched reference to one of these globals. Installs itself
 * only when {@link MOCK} is true; a real deployment (`VITE_GENARYX_API`
 * set) and a bare `vite preview` (neither flag set) are both untouched.
 */

const SANDBOX_MESSAGE =
  "Genaryx Live Demo: network access is disabled in this sandbox. Nothing this build does ever leaves your browser.";

let installed = false;

/** Idempotent: importing this module twice (or, under `vite dev`, a hot
 * reload of it) never re-wraps an already-patched global. */
export function installSandboxGuard(): void {
  if (!MOCK || installed) return;
  installed = true;

  guardFetch();
  guardXhr();
  guardWebSocket();
  guardEventSource();
  guardWebAuthn();
}

// ---------------------------------------------------------------------------
// fetch
// ---------------------------------------------------------------------------

function guardFetch(): void {
  if (typeof window === "undefined" || !("fetch" in window)) return;
  window.fetch = (() => Promise.reject(new Error(SANDBOX_MESSAGE))) as unknown as typeof window.fetch;
}

// ---------------------------------------------------------------------------
// XMLHttpRequest
// ---------------------------------------------------------------------------

/** Nothing in this app uses XHR (`transport.ts` is fetch/EventSource-only);
 * patched anyway as the same belt-and-suspenders the rest of this module
 * is. `open` silently records nothing rather than throwing (a
 * constructor-time throw would crash any caller that reasonably assumes
 * `new XMLHttpRequest()` + `.open()` never throws for a well-formed URL);
 * `send` fires an async `error` event, so a caller's own
 * `onerror`/`onreadystatechange` handling resolves instead of hanging
 * forever on a request that will never complete. */
function guardXhr(): void {
  if (typeof XMLHttpRequest === "undefined") return;
  const proto = XMLHttpRequest.prototype;
  proto.open = (() => {
    // Deliberately inert: recorded nowhere, sent nowhere.
  }) as unknown as typeof proto.open;
  proto.send = (function (this: XMLHttpRequest) {
    setTimeout(() => this.dispatchEvent(new Event("error")), 0);
  }) as unknown as typeof proto.send;
}

// ---------------------------------------------------------------------------
// WebSocket / EventSource
// ---------------------------------------------------------------------------

/** Shared shape for the two stub connection classes below: enough of the
 * real constructors' surface (readyState, on* handlers, add/removeEventListener
 * via `EventTarget`) that ordinary calling code, written for the real thing,
 * degrades the way it already has to for a real connection failure rather
 * than crashing outright. Never connects; asynchronously reports itself
 * errored exactly once, after construction, so a caller that attaches
 * `onerror` right after `new WebSocket(url)` (the normal pattern) still
 * sees it. */
class BlockedConnection extends EventTarget {
  readyState: number;
  readonly url: string;
  onopen: ((ev: Event) => void) | null = null;
  onmessage: ((ev: MessageEvent) => void) | null = null;
  onerror: ((ev: Event) => void) | null = null;

  constructor(url: string | URL, closedState: number) {
    super();
    this.url = String(url);
    this.readyState = closedState;
    setTimeout(() => {
      const err = new Event("error");
      this.dispatchEvent(err);
      this.onerror?.(err);
    }, 0);
  }

  close(): void {
    // Already closed; nothing to tear down.
  }
}

class BlockedWebSocket extends BlockedConnection {
  static readonly CONNECTING = 0;
  static readonly OPEN = 1;
  static readonly CLOSING = 2;
  static readonly CLOSED = 3;

  onclose: ((ev: CloseEvent) => void) | null = null;
  binaryType: BinaryType = "blob";
  readonly bufferedAmount = 0;
  readonly extensions = "";
  readonly protocol = "";

  constructor(url: string | URL, _protocols?: string | string[]) {
    super(url, BlockedWebSocket.CLOSED);
    setTimeout(() => {
      const closeEv = new CloseEvent("close", { wasClean: false, code: 1006, reason: SANDBOX_MESSAGE });
      this.dispatchEvent(closeEv);
      this.onclose?.(closeEv);
    }, 0);
  }

  send(): void {
    // Inert: never actually sent.
  }
}

class BlockedEventSource extends BlockedConnection {
  static readonly CONNECTING = 0;
  static readonly OPEN = 1;
  static readonly CLOSED = 2;

  withCredentials: boolean;

  constructor(url: string | URL, init?: EventSourceInit) {
    super(url, BlockedEventSource.CLOSED);
    this.withCredentials = init?.withCredentials ?? false;
  }
}

function guardWebSocket(): void {
  if (typeof window === "undefined" || !("WebSocket" in window)) return;
  window.WebSocket = BlockedWebSocket as unknown as typeof WebSocket;
}

function guardEventSource(): void {
  if (typeof window === "undefined" || !("EventSource" in window)) return;
  window.EventSource = BlockedEventSource as unknown as typeof EventSource;
}

// ---------------------------------------------------------------------------
// WebAuthn
// ---------------------------------------------------------------------------

/**
 * `navigator.credentials.create`/`.get` are how `lib/webauthn.ts` drives the
 * platform passkey prompt (Touch ID, Windows Hello, a security key). That
 * module already refuses to reach them under MOCK: `enrollPasskey` throws
 * before ever calling `.create` because `isWebShell()` is false, and
 * `invokeWithCeremony`'s ceremony branch never fires because its own
 * `listPasskeys()` probe resolves `webauthn_required: false` for the same
 * reason (both checks live in `lib/webauthn.ts`, both gated on the same
 * `isWebShell()` this demo also runs under, since `.env.mock` sets no
 * `VITE_GENARYX_API`). So today, nothing in this app can reach the real API
 * while `MOCK` is true, and that call site is outside this task's allowed
 * files besides.
 *
 * Patched here regardless, as the same belt-and-suspenders as the network
 * guards above: if a future change ever adds a call site that is not behind
 * `isWebShell()`, it resolves a faked assertion instead of ever reaching the
 * browser's own ceremony, so a demo visitor can never see an OS passkey
 * dialog, full stop, independent of whether every current call site stays
 * gated correctly. The fake resolves (never rejects, never hangs) with
 * well-shaped-but-empty buffers, so a caller that reads `.rawId`/
 * `.response.*` and base64-encodes them (exactly what `enrollPasskey`/
 * `ceremonyHeader` do) gets a harmless empty string rather than a crash.
 */
function guardWebAuthn(): void {
  if (typeof navigator === "undefined" || !navigator.credentials) return;

  const empty = new ArrayBuffer(0);
  const fakeCredential = {
    id: "demo-sandbox-no-real-passkey",
    rawId: empty,
    type: "public-key",
    response: {
      clientDataJSON: empty,
      attestationObject: empty,
      authenticatorData: empty,
      signature: empty,
      userHandle: empty,
    },
    getClientExtensionResults: () => ({}),
  };

  const fakeCreate = () => Promise.resolve(fakeCredential);
  const fakeGet = () => Promise.resolve(fakeCredential);

  try {
    // Own-property assignment on the `CredentialsContainer` instance shadows
    // the inherited prototype methods without needing to replace
    // `navigator.credentials` itself (which some engines expose as a
    // non-configurable accessor on `Navigator.prototype`).
    navigator.credentials.create = fakeCreate as unknown as typeof navigator.credentials.create;
    navigator.credentials.get = fakeGet as unknown as typeof navigator.credentials.get;
  } catch {
    // Some engine made `credentials` (or its methods) non-writable - nothing
    // more to do; the `isWebShell()` gates in `lib/webauthn.ts` are still in
    // force regardless, so this is a missed extra layer, not an open one.
  }
}

installSandboxGuard();
