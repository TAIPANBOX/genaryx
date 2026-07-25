import { invokeBackend, isWebShell, webApiBase } from "./transport";

/**
 * The WebAuthn per-action ceremony (D15 B3 part 2, docs/CONSOLE-IDP.md; the
 * wire contract mirrors `crates/web/src/webauthn.rs` and its `main.rs`
 * handlers field-for-field).
 *
 * Deployment truth, straight from the server module's own doc comment:
 * `navigator.credentials` exists only in a secure context, so the ceremony
 * works when this console is reached as `localhost` (the default loopback
 * bind, or an `ssh -L` forward over the operator's tunnel) or behind TLS.
 * Reaching it as a bare `http://10.x.x.x` has no WebAuthn at all - not a
 * degraded mode, just absent - and {@link webauthnAvailable} is how the UI
 * finds that out and says so honestly instead of silently downgrading.
 *
 * Three things live here:
 * - {@link enrollPasskey}: the registration ceremony
 *   (`navigator.credentials.create`), driven by `PasskeySettings`.
 * - {@link invokeWithCeremony}: the per-action ceremony
 *   (`navigator.credentials.get`) wrapped around a normal command dispatch -
 *   every sensitive command's wrapper (`lib/money.ts`'s `killRun`/
 *   `setBudget`, `lib/policy.ts`'s `decideApproval`) calls this instead of
 *   `invokeBackend` directly, so every existing caller inherits the
 *   ceremony with no panel-side change.
 * - {@link listPasskeys}: the one probe (`GET /api/webauthn/passkeys`) both
 *   `PasskeySettings` and `invokeWithCeremony` read, cached for the page's
 *   life and invalidated after a fresh enrollment.
 *
 * Not mocked: `dev:mock` has no server behind it to mint a challenge or
 * store a passkey, so every function here is a no-op (or a plain throw) when
 * {@link isWebShell} is false - the same guard `lib/session.ts`'s
 * `useSession` already uses for "no console session in this build".
 */

// ---------------------------------------------------------------------------
// base64url <-> ArrayBuffer
// ---------------------------------------------------------------------------

/** ArrayBuffer -> base64url, no padding - the exact string shape every field
 * of `x-genaryx-webauthn` and every `register/finish` field is sent as
 * (`crates/web/src/webauthn.rs` uses `URL_SAFE_NO_PAD` throughout). */
export function b64urlEncode(buf: ArrayBuffer): string {
  const bytes = new Uint8Array(buf);
  let binary = "";
  for (const b of bytes) binary += String.fromCharCode(b);
  return btoa(binary).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

/** base64url -> ArrayBuffer, tolerant of missing padding (the server never
 * pads either) - the inverse of {@link b64urlEncode}, used to decode every
 * challenge / `user.id` / `allowCredentials[].id` the backend hands back
 * into the `BufferSource` the browser's WebAuthn API wants. */
export function b64urlDecode(s: string): ArrayBuffer {
  const padded = s.replace(/-/g, "+").replace(/_/g, "/").padEnd(Math.ceil(s.length / 4) * 4, "=");
  const binary = atob(padded);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
  return bytes.buffer;
}

// ---------------------------------------------------------------------------
// availability
// ---------------------------------------------------------------------------

/** Whether this browser, in THIS context, can run a WebAuthn ceremony at
 * all - `navigator.credentials` only exists in a secure context (see this
 * module's own doc comment for the exact deployment shapes that satisfies).
 * `typeof window` is guarded so this reads as unavailable rather than
 * throwing in a non-browser context (a unit test under Node, say). */
export function webauthnAvailable(): boolean {
  return typeof window !== "undefined" && window.isSecureContext && "PublicKeyCredential" in window;
}

// ---------------------------------------------------------------------------
// raw REST helper (the four /webauthn/* endpoints are not "commands": no
// /command/<name>, no MOCK routing, so they cannot go through
// transport.ts's invokeBackend - same reasoning as lib/session.ts's own
// direct fetch of GET /auth/session)
// ---------------------------------------------------------------------------

/** Parse a REST response the same way `transport.ts`'s `invokeBackend`
 * parses a command response: read text, try JSON, fall back to the raw
 * text; reject with the parsed (or raw) body on a non-2xx rather than a
 * generic HTTP error, so callers can read the server's own `{error: ...}`
 * shape directly. */
async function fetchJson(url: string, init?: RequestInit): Promise<unknown> {
  const resp = await fetch(url, { credentials: "include", ...init });
  const text = await resp.text();
  let body: unknown = null;
  if (text) {
    try {
      body = JSON.parse(text);
    } catch {
      body = text;
    }
  }
  if (!resp.ok) throw body;
  return body;
}

// ---------------------------------------------------------------------------
// the passkeys probe
// ---------------------------------------------------------------------------

/** Mirrors one entry of `GET /api/webauthn/passkeys`'s `passkeys` array
 * (`crates/web/src/main.rs`'s `webauthn_list` - public metadata only, never
 * the public key). */
export interface PasskeyInfo {
  credential_id: string;
  label: string;
  created_at: string;
}

/** Mirrors `GET /api/webauthn/passkeys`'s full body. */
export interface PasskeysProbe {
  passkeys: PasskeyInfo[];
  webauthn_required: boolean;
}

const NO_PASSKEYS_PROBE: PasskeysProbe = { passkeys: [], webauthn_required: false };

let passkeysCache: Promise<PasskeysProbe> | null = null;

async function fetchPasskeysProbe(): Promise<PasskeysProbe> {
  if (!isWebShell()) return NO_PASSKEYS_PROBE;
  return (await fetchJson(`${webApiBase()}/webauthn/passkeys`)) as PasskeysProbe;
}

/**
 * `GET /api/webauthn/passkeys`, cached for the life of the page - every
 * caller (this module's own {@link invokeWithCeremony} and
 * `PasskeySettings`) shares the one in-flight/settled request rather than
 * re-probing on every privileged action. Not web-shell (no server behind
 * `dev:mock` or a bare preview): resolves to the vacuous "nothing enrolled,
 * no ceremony" shape without ever touching the network.
 *
 * A failed probe clears its own cache entry before rejecting, so a
 * transient failure does not wedge the page shut for good - the next
 * caller gets a fresh attempt rather than the same stale rejection forever.
 */
export function listPasskeys(): Promise<PasskeysProbe> {
  if (!passkeysCache) {
    passkeysCache = fetchPasskeysProbe().catch((err: unknown) => {
      passkeysCache = null;
      throw err;
    });
  }
  return passkeysCache;
}

/** Call after a successful {@link enrollPasskey} (or whenever the enrolled
 * set might have changed) so the next {@link listPasskeys} re-probes
 * instead of answering from a now-stale cache. */
export function invalidatePasskeysCache(): void {
  passkeysCache = null;
}

// ---------------------------------------------------------------------------
// registration (navigator.credentials.create)
// ---------------------------------------------------------------------------

/** Thrown when the operator dismissed their platform's own passkey prompt
 * (Touch ID, Windows Hello, a security key's own cancel, ...) - the
 * `NotAllowedError` `DOMException` `navigator.credentials.create`/`.get`
 * both raise on a plain change of mind. Never a real failure: there was no
 * server round trip to fail, so it carries no server text - catching this
 * specific shape is how a caller stays quiet instead of showing an error
 * banner for what is simply "not right now". */
export class CeremonyCancelled extends Error {
  constructor() {
    super("cancelled");
    this.name = "CeremonyCancelled";
  }
}

/** Not `err instanceof Error`: a browser's `NotAllowedError` is a
 * `DOMException`, and whether that inherits from `Error` is not something
 * to depend on across engines - `.name` is the one property every
 * implementation guarantees, and is the standard way WebAuthn code
 * recognizes this specific cancel. */
function isNotAllowed(err: unknown): boolean {
  return Boolean(err) && typeof err === "object" && (err as { name?: unknown }).name === "NotAllowedError";
}

/** The shape `POST /api/webauthn/register/start` answers with: almost a
 * `PublicKeyCredentialCreationOptions`, except `challenge` and `user.id` are
 * base64url strings (JSON has no bytes) rather than the `BufferSource` the
 * browser API wants - decoded below before the spread into
 * `navigator.credentials.create`. */
interface RegisterStartDto {
  challenge: string;
  rp: PublicKeyCredentialRpEntity;
  user: { id: string; name: string; displayName: string };
  pubKeyCredParams: PublicKeyCredentialParameters[];
  timeout: number;
  attestation: AttestationConveyancePreference;
  authenticatorSelection: AuthenticatorSelectionCriteria;
}

/** Mirrors `POST /api/webauthn/register/finish`'s success body. */
export interface EnrolledPasskey {
  enrolled: true;
  credential_id: string;
}

/**
 * Register a new passkey in the current session
 * (`navigator.credentials.create`) and enroll it
 * (`POST /api/webauthn/register/finish`). Invalidates {@link listPasskeys}'s
 * cache on success, so the next probe sees it without a page reload.
 *
 * Rejects with the server's `{error: ...}` body, unmodified, for a genuine
 * refusal (a bad ceremony, a store failure, ...); a plain operator cancel
 * rejects with {@link CeremonyCancelled} instead.
 */
export async function enrollPasskey(label?: string): Promise<EnrolledPasskey> {
  if (!isWebShell()) {
    throw new Error("no backend: cannot enroll a passkey without a console session");
  }
  if (!webauthnAvailable()) {
    throw new Error(
      "WebAuthn is not available in this context: reach the console as localhost (loopback, or an ssh -L tunnel) or behind TLS",
    );
  }

  const options = (await fetchJson(`${webApiBase()}/webauthn/register/start`, {
    method: "POST",
  })) as RegisterStartDto;

  let credential: PublicKeyCredential | null;
  try {
    credential = (await navigator.credentials.create({
      publicKey: {
        challenge: b64urlDecode(options.challenge),
        rp: options.rp,
        user: {
          id: b64urlDecode(options.user.id),
          name: options.user.name,
          displayName: options.user.displayName,
        },
        pubKeyCredParams: options.pubKeyCredParams,
        timeout: options.timeout,
        attestation: options.attestation,
        authenticatorSelection: options.authenticatorSelection,
      },
    })) as PublicKeyCredential | null;
  } catch (err) {
    if (isNotAllowed(err)) throw new CeremonyCancelled();
    throw err;
  }
  if (!credential) throw new CeremonyCancelled();

  const response = credential.response as AuthenticatorAttestationResponse;
  const result = (await fetchJson(`${webApiBase()}/webauthn/register/finish`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({
      label: label ?? "",
      credential_id: b64urlEncode(credential.rawId),
      client_data_json: b64urlEncode(response.clientDataJSON),
      attestation_object: b64urlEncode(response.attestationObject),
    }),
  })) as EnrolledPasskey;

  invalidatePasskeysCache();
  return result;
}

// ---------------------------------------------------------------------------
// per-action ceremony (navigator.credentials.get)
// ---------------------------------------------------------------------------

/** The shape `POST /api/webauthn/action/start` answers with (ids base64url,
 * decoded below for `allowCredentials`). */
interface ActionStartDto {
  challenge: string;
  rp_id: string;
  timeout: number;
  user_verification: UserVerificationRequirement;
  allow_credentials: { type: "public-key"; id: string }[];
}

/** The envelope this module builds for `x-genaryx-webauthn`, mirroring
 * `crates/web/src/main.rs`'s `AssertionEnvelope` field-for-field: every
 * field is base64url of the raw bytes the browser handed back. */
interface AssertionEnvelope {
  credential_id: string;
  client_data_json: string;
  authenticator_data: string;
  signature: string;
}

/**
 * Run the per-action ceremony for `command`/`args`
 * (`POST /api/webauthn/action/start` -> `navigator.credentials.get`) and
 * return the `x-genaryx-webauthn` header value: base64url of the JSON
 * envelope the gate decodes (`crates/web/src/main.rs`'s `webauthn_gate`).
 *
 * `args` must be the EXACT object the caller then dispatches with - the
 * server binds the challenge to a hash of it (`args_sha256`), so a
 * mismatch here is a guaranteed refusal, by design (replay-proofing, not a
 * bug to work around). Rejects with the server's `{error: ...}` body for a
 * genuine refusal, or {@link CeremonyCancelled} for a plain operator
 * cancel.
 */
export async function ceremonyHeader(command: string, args: Record<string, unknown>): Promise<string> {
  if (!webauthnAvailable()) {
    throw new Error(
      "WebAuthn is not available in this context: reach the console as localhost (loopback, or an ssh -L tunnel) or behind TLS",
    );
  }

  const start = (await fetchJson(`${webApiBase()}/webauthn/action/start`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ command, args }),
  })) as ActionStartDto;

  let assertion: PublicKeyCredential | null;
  try {
    assertion = (await navigator.credentials.get({
      publicKey: {
        challenge: b64urlDecode(start.challenge),
        rpId: start.rp_id,
        timeout: start.timeout,
        userVerification: start.user_verification,
        allowCredentials: start.allow_credentials.map((c) => ({
          type: c.type,
          id: b64urlDecode(c.id),
        })),
      },
    })) as PublicKeyCredential | null;
  } catch (err) {
    if (isNotAllowed(err)) throw new CeremonyCancelled();
    throw err;
  }
  if (!assertion) throw new CeremonyCancelled();

  const response = assertion.response as AuthenticatorAssertionResponse;
  const envelope: AssertionEnvelope = {
    credential_id: b64urlEncode(assertion.rawId),
    client_data_json: b64urlEncode(response.clientDataJSON),
    authenticator_data: b64urlEncode(response.authenticatorData),
    signature: b64urlEncode(response.signature),
  };
  return b64urlEncode(new TextEncoder().encode(JSON.stringify(envelope)).buffer);
}

// ---------------------------------------------------------------------------
// invokeWithCeremony: the one call site lib/money.ts + lib/policy.ts use
// ---------------------------------------------------------------------------

/** True when `err` is `genaryx-web`'s "a webauthn assertion is required for
 * this command" refusal - the `428 {"error": ..., "webauthn": "required"}`
 * `webauthn_gate` returns when the caller has a passkey enrolled but sent
 * no `x-genaryx-webauthn` header (`crates/web/src/main.rs`). This is the
 * ONE shape {@link invokeWithCeremony} retries on; every other rejection
 * (a genuine `MoneyError`/`PolicyError`, a bad-signature 403, ...) passes
 * straight through unchanged. */
function ceremonyRequired(err: unknown): boolean {
  return Boolean(err) && typeof err === "object" && (err as { webauthn?: unknown }).webauthn === "required";
}

/** Coerce a plain `{error: string}` REST rejection (every non-command
 * `/webauthn/*` endpoint's own failure shape, and the command gate's own
 * 403 body - none of which carry a `kind`) into a proper `Error`, so an
 * UNMODIFIED `toMoneyError`/`toPolicyError` fallback (`err instanceof Error
 * ? err.message : String(err)`) already renders the server's exact text
 * instead of "[object Object]" - no change needed to either normalizer. A
 * genuine domain error (`MoneyError`/`PolicyError`, always tagged with its
 * own `kind`) is returned unchanged, and so is anything already an `Error`
 * (e.g. {@link CeremonyCancelled}). */
function asError(err: unknown): unknown {
  if (
    err &&
    typeof err === "object" &&
    !("kind" in err) &&
    typeof (err as { error?: unknown }).error === "string"
  ) {
    return new Error((err as { error: string }).error);
  }
  return err;
}

/** Run the ceremony for `command`/`args` and dispatch once with its header,
 * applying {@link asError} to whatever escapes so a ceremony-stage failure
 * (the `action/start` probe, or the gate's own 403) reads as plain text
 * through the existing error banners rather than a stringified object. */
async function runCeremonyAndDispatch<T>(
  command: string,
  args: Record<string, unknown> | undefined,
  effectiveArgs: Record<string, unknown>,
): Promise<T> {
  try {
    const header = await ceremonyHeader(command, effectiveArgs);
    return await invokeBackend<T>(command, args, { "x-genaryx-webauthn": header });
  } catch (err) {
    throw asError(err);
  }
}

/**
 * Dispatch a sensitive command through the per-action WebAuthn ceremony
 * (docs/CONSOLE-IDP.md B3/2; the three commands in
 * `crates/web/src/main.rs`'s `SENSITIVE_COMMANDS`).
 *
 * Two paths, matching `webauthn_gate`'s own two honest outcomes:
 * - The common case: {@link listPasskeys}'s cached probe already says a
 *   ceremony is required, and this browser can run one - get the header
 *   FIRST, then dispatch once, with it.
 * - Everything else (nothing enrolled yet, the probe has not resolved
 *   favorably, or this context cannot run WebAuthn at all): dispatch
 *   plainly first. If the server still comes back with the 428 "required"
 *   shape (the probe was stale, or a passkey was enrolled from another
 *   tab), run the ceremony once and retry ONCE - never a loop, matching the
 *   server's own challenge being one-shot.
 *
 * Callers never see `invokeBackend`'s plain command surface for these
 * commands - `lib/money.ts`'s `killRun`/`setBudget`, `lib/policy.ts`'s
 * `decideApproval` and `lib/remote.ts`'s `issueOperatorWgConfig`/
 * `revokeOperatorWgPeer` call this instead, so every existing caller of those
 * wrappers inherits the ceremony with no panel-side change. The two WireGuard
 * ones are here for the same reason a kill is: issuing a peer mints a road
 * into the control plane and revoking one cuts an operator off mid-incident.
 */
export async function invokeWithCeremony<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  const effectiveArgs = args ?? {};
  const probe = await listPasskeys().catch(() => NO_PASSKEYS_PROBE);

  if (probe.webauthn_required && webauthnAvailable()) {
    return runCeremonyAndDispatch<T>(command, args, effectiveArgs);
  }

  try {
    return await invokeBackend<T>(command, args);
  } catch (err) {
    if (!ceremonyRequired(err)) throw err;
    return runCeremonyAndDispatch<T>(command, args, effectiveArgs);
  }
}
