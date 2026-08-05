import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

// `webauthn.ts` imports these three from `./transport`; mocking the module
// here (hoisted by vitest above every import in this file) is what lets
// `invokeWithCeremony`'s tests drive the retry-on-428 path without a real
// backend, matching every other transport-level test's own "mocked
// transport" convention for this repo (none needed one before, since this
// is the first suite whose subject calls through `transport.ts`).
vi.mock("./transport", () => ({
  isWebShell: vi.fn(),
  webApiBase: vi.fn(),
  invokeBackend: vi.fn(),
}));

import { invokeBackend, isWebShell, webApiBase } from "./transport";
import {
  CeremonyCancelled,
  b64urlDecode,
  b64urlEncode,
  ceremonyHeader,
  enrollPasskey,
  invalidatePasskeysCache,
  invokeWithCeremony,
  listPasskeys,
  operatorPasswordRequired,
  removePasskey,
  webauthnAvailable,
} from "./webauthn";

// ---------------------------------------------------------------------------
// Shared fixtures / helpers.
// ---------------------------------------------------------------------------

/** A minimal fetch `Response`-shaped object: `fetchJson` (webauthn.ts) only
 * ever reads `.ok` and `.text()`. */
function jsonResponse(body: unknown, ok = true): unknown {
  return { ok, text: async () => JSON.stringify(body) };
}

/** A fake `fetch` that answers by URL suffix - enough for every endpoint
 * this module calls, each with one fixed response per test. */
function routedFetch(routes: Record<string, unknown>) {
  return vi.fn(async (url: string) => {
    for (const [suffix, body] of Object.entries(routes)) {
      if (url.endsWith(suffix)) return jsonResponse(body);
    }
    throw new Error(`unexpected fetch url in test: ${url}`);
  });
}

/** A secure-context `window` stub - just enough for `webauthnAvailable()`. */
function secureWindow() {
  return { isSecureContext: true, PublicKeyCredential: function PublicKeyCredential() {} };
}

beforeEach(() => {
  vi.resetAllMocks();
  invalidatePasskeysCache();
  vi.mocked(webApiBase).mockReturnValue("/api");
});

afterEach(() => {
  vi.unstubAllGlobals();
});

// ---------------------------------------------------------------------------
// b64url round trip
// ---------------------------------------------------------------------------

describe("b64urlEncode / b64urlDecode", () => {
  it("round-trips byte sequences of every padding remainder (0, 1, 2, 3 bytes mod 3)", () => {
    const cases = [
      [],
      [0],
      [1, 2],
      [1, 2, 3],
      [1, 2, 3, 4],
      Array.from({ length: 37 }, (_, i) => (i * 7) % 256),
    ];
    for (const bytes of cases) {
      const encoded = b64urlEncode(new Uint8Array(bytes).buffer);
      const decoded = Array.from(new Uint8Array(b64urlDecode(encoded)));
      expect(decoded).toEqual(bytes);
    }
  });

  it("never emits '+', '/' or '=' - the whole point of base64url over plain base64", () => {
    const encoded = b64urlEncode(new Uint8Array([251, 239, 190]).buffer);
    expect(encoded).not.toMatch(/[+/=]/);
  });

  it("encodes to the exact known base64url string (no padding)", () => {
    expect(b64urlEncode(new TextEncoder().encode("hello").buffer)).toBe("aGVsbG8");
  });

  it("decodes a known unpadded base64url string back to the exact bytes", () => {
    const decoded = new Uint8Array(b64urlDecode("aGVsbG8"));
    expect(new TextDecoder().decode(decoded)).toBe("hello");
  });

  it("decodes the empty string to an empty buffer rather than throwing", () => {
    expect(b64urlDecode("").byteLength).toBe(0);
  });
});

// ---------------------------------------------------------------------------
// webauthnAvailable
// ---------------------------------------------------------------------------

describe("webauthnAvailable", () => {
  it("is false with no window at all, matching this test's own Node environment", () => {
    expect(webauthnAvailable()).toBe(false);
  });

  it("is true in a secure context with PublicKeyCredential present", () => {
    vi.stubGlobal("window", secureWindow());
    expect(webauthnAvailable()).toBe(true);
  });

  it("is false when the context is not secure, even with PublicKeyCredential present", () => {
    vi.stubGlobal("window", { isSecureContext: false, PublicKeyCredential: function () {} });
    expect(webauthnAvailable()).toBe(false);
  });

  it("is false without PublicKeyCredential, even in a secure context", () => {
    vi.stubGlobal("window", { isSecureContext: true });
    expect(webauthnAvailable()).toBe(false);
  });
});

// ---------------------------------------------------------------------------
// listPasskeys / invalidatePasskeysCache
// ---------------------------------------------------------------------------

describe("listPasskeys", () => {
  it("resolves to the vacuous no-ceremony shape without touching the network outside a web shell", async () => {
    vi.mocked(isWebShell).mockReturnValue(false);
    const fetchMock = vi.fn();
    vi.stubGlobal("fetch", fetchMock);

    await expect(listPasskeys()).resolves.toEqual({
      passkeys: [],
      webauthn_required: false,
      policy_requires_passkey: false,
    });
    expect(fetchMock).not.toHaveBeenCalled();
  });

  it("caches the probe for the page's life - a second call does not refetch", async () => {
    vi.mocked(isWebShell).mockReturnValue(true);
    const probe = {
      passkeys: [{ credential_id: "c1", label: "x", created_at: "2026-01-01T00:00:00Z" }],
      webauthn_required: true,
    };
    const fetchMock = routedFetch({ "/webauthn/passkeys": probe });
    vi.stubGlobal("fetch", fetchMock);

    await expect(listPasskeys()).resolves.toEqual(probe);
    await expect(listPasskeys()).resolves.toEqual(probe);
    expect(fetchMock).toHaveBeenCalledTimes(1);
  });

  it("clears its cache on failure, so the next call retries instead of repeating the same rejection", async () => {
    vi.mocked(isWebShell).mockReturnValue(true);
    const fetchMock = vi
      .fn()
      .mockResolvedValueOnce(jsonResponse({ error: "passkey store unavailable: disk full" }, false))
      .mockResolvedValueOnce(jsonResponse({ passkeys: [], webauthn_required: false }));
    vi.stubGlobal("fetch", fetchMock);

    await expect(listPasskeys()).rejects.toEqual({ error: "passkey store unavailable: disk full" });
    await expect(listPasskeys()).resolves.toEqual({ passkeys: [], webauthn_required: false });
    expect(fetchMock).toHaveBeenCalledTimes(2);
  });

  it("invalidatePasskeysCache forces a fresh probe even after a settled, successful one", async () => {
    vi.mocked(isWebShell).mockReturnValue(true);
    const fetchMock = routedFetch({ "/webauthn/passkeys": { passkeys: [], webauthn_required: false } });
    vi.stubGlobal("fetch", fetchMock);

    await listPasskeys();
    invalidatePasskeysCache();
    await listPasskeys();
    expect(fetchMock).toHaveBeenCalledTimes(2);
  });
});

// ---------------------------------------------------------------------------
// enrollPasskey
// ---------------------------------------------------------------------------

describe("enrollPasskey", () => {
  beforeEach(() => {
    vi.mocked(isWebShell).mockReturnValue(true);
    vi.stubGlobal("window", secureWindow());
  });

  it("decodes register/start's challenge and user.id, passes the rest through, and posts register/finish with base64url response fields", async () => {
    const challengeBytes = new Uint8Array([1, 2, 3, 4, 5]);
    const userIdBytes = new TextEncoder().encode("alice");
    const startDto = {
      challenge: b64urlEncode(challengeBytes.buffer),
      rp: { id: "localhost", name: "Genaryx" },
      user: { id: b64urlEncode(userIdBytes.buffer), name: "alice", displayName: "alice" },
      pubKeyCredParams: [{ type: "public-key", alg: -7 }],
      timeout: 120000,
      attestation: "none",
      authenticatorSelection: { userVerification: "preferred" },
    };
    const finishDto = { enrolled: true, credential_id: "cred-xyz" };
    let finishBody: unknown;

    vi.stubGlobal(
      "fetch",
      vi.fn(async (url: string, init?: RequestInit) => {
        if (url.endsWith("/webauthn/register/start")) return jsonResponse(startDto);
        if (url.endsWith("/webauthn/register/finish")) {
          finishBody = JSON.parse(init!.body as string);
          return jsonResponse(finishDto);
        }
        throw new Error(`unexpected fetch url in test: ${url}`);
      }),
    );

    const rawId = new TextEncoder().encode("cred-xyz").buffer;
    const clientDataJSON = new TextEncoder().encode('{"type":"webauthn.create"}').buffer;
    const attestationObject = new Uint8Array([9, 9]).buffer;
    let createOptions: CredentialCreationOptions | undefined;
    vi.stubGlobal("navigator", {
      credentials: {
        create: vi.fn(async (options: CredentialCreationOptions) => {
          createOptions = options;
          return { rawId, response: { clientDataJSON, attestationObject } };
        }),
      },
    });

    await expect(enrollPasskey("MacBook")).resolves.toEqual(finishDto);

    const publicKey = createOptions!.publicKey!;
    expect(new Uint8Array(publicKey.challenge as ArrayBuffer)).toEqual(challengeBytes);
    expect(new Uint8Array(publicKey.user!.id as ArrayBuffer)).toEqual(userIdBytes);
    expect(publicKey.pubKeyCredParams).toEqual(startDto.pubKeyCredParams);
    expect(publicKey.rp).toEqual(startDto.rp);

    expect(finishBody).toEqual({
      label: "MacBook",
      credential_id: b64urlEncode(rawId),
      client_data_json: b64urlEncode(clientDataJSON),
      attestation_object: b64urlEncode(attestationObject),
    });
  });

  it("sends an empty label when none is given", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async (url: string, init?: RequestInit) => {
        if (url.endsWith("/webauthn/register/start")) {
          return jsonResponse({
            challenge: b64urlEncode(new Uint8Array([1]).buffer),
            rp: { id: "localhost", name: "Genaryx" },
            user: { id: b64urlEncode(new Uint8Array([1]).buffer), name: "a", displayName: "a" },
            pubKeyCredParams: [{ type: "public-key", alg: -7 }],
            timeout: 1,
            attestation: "none",
            authenticatorSelection: {},
          });
        }
        expect(JSON.parse(init!.body as string)).toMatchObject({ label: "" });
        return jsonResponse({ enrolled: true, credential_id: "c" });
      }),
    );
    vi.stubGlobal("navigator", {
      credentials: {
        create: vi.fn(async () => ({
          rawId: new Uint8Array([1]).buffer,
          response: { clientDataJSON: new Uint8Array([1]).buffer, attestationObject: new Uint8Array([1]).buffer },
        })),
      },
    });

    await enrollPasskey();
  });

  it("sends the operator password to register/start, the factor a first enrollment needs", async () => {
    let startBody: unknown;
    vi.stubGlobal(
      "fetch",
      vi.fn(async (url: string, init?: RequestInit) => {
        if (url.endsWith("/webauthn/register/start")) {
          startBody = JSON.parse(init!.body as string);
          return jsonResponse({
            challenge: b64urlEncode(new Uint8Array([1]).buffer),
            rp: { id: "localhost", name: "Genaryx" },
            user: { id: b64urlEncode(new Uint8Array([1]).buffer), name: "a", displayName: "a" },
            pubKeyCredParams: [{ type: "public-key", alg: -7 }],
            timeout: 1,
            attestation: "none",
            authenticatorSelection: {},
          });
        }
        return jsonResponse({ enrolled: true, credential_id: "c" });
      }),
    );
    vi.stubGlobal("navigator", {
      credentials: {
        create: vi.fn(async () => ({
          rawId: new Uint8Array([1]).buffer,
          response: { clientDataJSON: new Uint8Array([1]).buffer, attestationObject: new Uint8Array([1]).buffer },
        })),
        get: vi.fn(),
      },
    });

    await enrollPasskey("Yubikey", "correct horse battery");

    expect(startBody).toEqual({ operator_password: "correct horse battery" });
  });

  it("on an assertion_required refusal, confirms with an enrolled passkey and starts once more, with the header", async () => {
    let startCalls = 0;
    let secondStartHeaders: Record<string, string> | undefined;
    let actionStartBody: unknown;
    vi.stubGlobal(
      "fetch",
      vi.fn(async (url: string, init?: RequestInit) => {
        if (url.endsWith("/webauthn/register/start")) {
          startCalls += 1;
          if (startCalls === 1) {
            return jsonResponse(
              { error: "you already have an enrolled passkey", webauthn: "assertion_required" },
              false,
            );
          }
          secondStartHeaders = init!.headers as Record<string, string>;
          return jsonResponse({
            challenge: b64urlEncode(new Uint8Array([1]).buffer),
            rp: { id: "localhost", name: "Genaryx" },
            user: { id: b64urlEncode(new Uint8Array([1]).buffer), name: "a", displayName: "a" },
            pubKeyCredParams: [{ type: "public-key", alg: -7 }],
            timeout: 1,
            attestation: "none",
            authenticatorSelection: {},
          });
        }
        if (url.endsWith("/webauthn/action/start")) {
          actionStartBody = JSON.parse(init!.body as string);
          return jsonResponse({
            challenge: b64urlEncode(new Uint8Array([2]).buffer),
            rp_id: "localhost",
            timeout: 1,
            user_verification: "preferred",
            allow_credentials: [],
          });
        }
        return jsonResponse({ enrolled: true, credential_id: "cred-2" });
      }),
    );
    vi.stubGlobal("navigator", {
      credentials: {
        get: vi.fn(async () => ({
          rawId: new Uint8Array([1]).buffer,
          response: {
            clientDataJSON: new Uint8Array([1]).buffer,
            authenticatorData: new Uint8Array([1]).buffer,
            signature: new Uint8Array([1]).buffer,
          },
        })),
        create: vi.fn(async () => ({
          rawId: new Uint8Array([2]).buffer,
          response: { clientDataJSON: new Uint8Array([1]).buffer, attestationObject: new Uint8Array([1]).buffer },
        })),
      },
    });

    await expect(enrollPasskey("second key")).resolves.toEqual({
      enrolled: true,
      credential_id: "cred-2",
    });
    expect(startCalls).toBe(2);
    expect(actionStartBody).toEqual({ command: "webauthn_enroll_passkey", args: {} });
    expect(secondStartHeaders!["x-genaryx-webauthn"]).toBeTruthy();
  });

  it("rejects with CeremonyCancelled on a NotAllowedError from navigator.credentials.create", async () => {
    vi.stubGlobal(
      "fetch",
      routedFetch({
        "/webauthn/register/start": {
          challenge: b64urlEncode(new Uint8Array([1]).buffer),
          rp: { id: "localhost", name: "Genaryx" },
          user: { id: b64urlEncode(new Uint8Array([1]).buffer), name: "a", displayName: "a" },
          pubKeyCredParams: [{ type: "public-key", alg: -7 }],
          timeout: 1,
          attestation: "none",
          authenticatorSelection: {},
        },
      }),
    );
    const notAllowed = Object.assign(new Error("dismissed"), { name: "NotAllowedError" });
    vi.stubGlobal("navigator", { credentials: { create: vi.fn().mockRejectedValue(notAllowed) } });

    await expect(enrollPasskey()).rejects.toBeInstanceOf(CeremonyCancelled);
  });

  it("rejects with the server's raw {error} body on a register/finish refusal, unmodified", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(async (url: string) => {
        if (url.endsWith("/webauthn/register/start")) {
          return jsonResponse({
            challenge: b64urlEncode(new Uint8Array([1]).buffer),
            rp: { id: "localhost", name: "Genaryx" },
            user: { id: b64urlEncode(new Uint8Array([1]).buffer), name: "a", displayName: "a" },
            pubKeyCredParams: [{ type: "public-key", alg: -7 }],
            timeout: 1,
            attestation: "none",
            authenticatorSelection: {},
          });
        }
        return jsonResponse({ error: "webauthn: credential id mismatch" }, false);
      }),
    );
    vi.stubGlobal("navigator", {
      credentials: {
        create: vi.fn(async () => ({
          rawId: new Uint8Array([1]).buffer,
          response: { clientDataJSON: new Uint8Array([1]).buffer, attestationObject: new Uint8Array([1]).buffer },
        })),
      },
    });

    await expect(enrollPasskey()).rejects.toEqual({ error: "webauthn: credential id mismatch" });
  });

  it("throws a plain Error, never CeremonyCancelled, with no web-shell backend to enroll against", async () => {
    vi.mocked(isWebShell).mockReturnValue(false);
    await expect(enrollPasskey()).rejects.toThrow(/no backend/);
  });

  it("throws a plain Error, never CeremonyCancelled, when this context cannot run WebAuthn at all", async () => {
    vi.stubGlobal("window", { isSecureContext: false });
    await expect(enrollPasskey()).rejects.toThrow(/WebAuthn is not available/);
  });
});

// ---------------------------------------------------------------------------
// removePasskey
// ---------------------------------------------------------------------------

describe("removePasskey", () => {
  beforeEach(() => {
    vi.mocked(isWebShell).mockReturnValue(true);
    vi.stubGlobal("window", secureWindow());
  });

  it("confirms with an enrolled passkey, binding the ceremony to the credential being removed", async () => {
    let actionStartBody: unknown;
    let removeInit: RequestInit | undefined;
    vi.stubGlobal(
      "fetch",
      vi.fn(async (url: string, init?: RequestInit) => {
        if (url.endsWith("/webauthn/action/start")) {
          actionStartBody = JSON.parse(init!.body as string);
          return jsonResponse({
            challenge: b64urlEncode(new Uint8Array([1]).buffer),
            rp_id: "localhost",
            timeout: 1,
            user_verification: "preferred",
            allow_credentials: [],
          });
        }
        removeInit = init;
        return jsonResponse({ removed: true, credential_id: "cred-2", remaining: 1 });
      }),
    );
    vi.stubGlobal("navigator", {
      credentials: {
        get: vi.fn(async () => ({
          rawId: new Uint8Array([1]).buffer,
          response: {
            clientDataJSON: new Uint8Array([1]).buffer,
            authenticatorData: new Uint8Array([1]).buffer,
            signature: new Uint8Array([1]).buffer,
          },
        })),
      },
    });

    await expect(removePasskey("cred-2")).resolves.toEqual({
      removed: true,
      credential_id: "cred-2",
      remaining: 1,
    });
    expect(actionStartBody).toEqual({
      command: "webauthn_remove_passkey",
      args: { credential_id: "cred-2" },
    });
    expect(JSON.parse(removeInit!.body as string)).toEqual({ credential_id: "cred-2" });
    expect((removeInit!.headers as Record<string, string>)["x-genaryx-webauthn"]).toBeTruthy();
  });

  it("sends the operator password instead, and never touches WebAuthn, when one is given", async () => {
    let removeInit: RequestInit | undefined;
    vi.stubGlobal(
      "fetch",
      vi.fn(async (url: string, init?: RequestInit) => {
        if (!url.endsWith("/webauthn/passkeys/remove")) throw new Error(`unexpected fetch url: ${url}`);
        removeInit = init;
        return jsonResponse({ removed: true, credential_id: "cred-1", remaining: 0 });
      }),
    );
    const get = vi.fn();
    vi.stubGlobal("navigator", { credentials: { get } });

    await removePasskey("cred-1", "correct horse battery");

    expect(JSON.parse(removeInit!.body as string)).toEqual({
      credential_id: "cred-1",
      operator_password: "correct horse battery",
    });
    expect(get).not.toHaveBeenCalled();
  });

  it("recognizes the server's password_required refusal, so the UI can ask for it", () => {
    expect(operatorPasswordRequired({ error: "last passkey", webauthn: "password_required" })).toBe(true);
    expect(operatorPasswordRequired({ error: "nope", webauthn: "assertion_required" })).toBe(false);
    expect(operatorPasswordRequired(null)).toBe(false);
  });
});

// ---------------------------------------------------------------------------
// ceremonyHeader
// ---------------------------------------------------------------------------

describe("ceremonyHeader", () => {
  beforeEach(() => {
    vi.stubGlobal("window", secureWindow());
  });

  it("posts action/start with the command and args, decodes the challenge and allowCredentials for navigator.credentials.get, and returns a header that decodes to the exact envelope", async () => {
    const challengeBytes = new Uint8Array([9, 9, 9]);
    const credIdBytes = new TextEncoder().encode("cred-1");
    const startDto = {
      challenge: b64urlEncode(challengeBytes.buffer),
      rp_id: "localhost",
      timeout: 120000,
      user_verification: "preferred",
      allow_credentials: [{ type: "public-key" as const, id: b64urlEncode(credIdBytes.buffer) }],
    };
    let actionStartBody: unknown;
    vi.stubGlobal(
      "fetch",
      vi.fn(async (url: string, init?: RequestInit) => {
        if (url.endsWith("/webauthn/action/start")) {
          actionStartBody = JSON.parse(init!.body as string);
          return jsonResponse(startDto);
        }
        throw new Error(`unexpected fetch url in test: ${url}`);
      }),
    );

    const rawId = credIdBytes.buffer;
    const clientDataJSON = new TextEncoder().encode('{"type":"webauthn.get"}').buffer;
    const authenticatorData = new Uint8Array([1, 2, 3]).buffer;
    const signature = new Uint8Array([4, 5, 6]).buffer;
    let getOptions: CredentialRequestOptions | undefined;
    vi.stubGlobal("navigator", {
      credentials: {
        get: vi.fn(async (options: CredentialRequestOptions) => {
          getOptions = options;
          return { rawId, response: { clientDataJSON, authenticatorData, signature } };
        }),
      },
    });

    const header = await ceremonyHeader("money_kill_run", { run_id: "a" });

    expect(actionStartBody).toEqual({ command: "money_kill_run", args: { run_id: "a" } });

    const publicKey = getOptions!.publicKey!;
    expect(new Uint8Array(publicKey.challenge as ArrayBuffer)).toEqual(challengeBytes);
    expect(publicKey.rpId).toBe("localhost");
    expect(new Uint8Array(publicKey.allowCredentials![0]!.id as ArrayBuffer)).toEqual(credIdBytes);

    const envelope = JSON.parse(new TextDecoder().decode(b64urlDecode(header)));
    expect(envelope).toEqual({
      credential_id: b64urlEncode(rawId),
      client_data_json: b64urlEncode(clientDataJSON),
      authenticator_data: b64urlEncode(authenticatorData),
      signature: b64urlEncode(signature),
    });
  });

  it("rejects with CeremonyCancelled on a NotAllowedError from navigator.credentials.get", async () => {
    vi.stubGlobal(
      "fetch",
      routedFetch({
        "/webauthn/action/start": {
          challenge: b64urlEncode(new Uint8Array([1]).buffer),
          rp_id: "localhost",
          timeout: 1,
          user_verification: "preferred",
          allow_credentials: [],
        },
      }),
    );
    const notAllowed = Object.assign(new Error("dismissed"), { name: "NotAllowedError" });
    vi.stubGlobal("navigator", { credentials: { get: vi.fn().mockRejectedValue(notAllowed) } });

    await expect(ceremonyHeader("money_kill_run", {})).rejects.toBeInstanceOf(CeremonyCancelled);
  });

  it("throws a plain Error, never touching the network, when this context cannot run WebAuthn at all", async () => {
    vi.stubGlobal("window", { isSecureContext: false });
    const fetchMock = vi.fn();
    vi.stubGlobal("fetch", fetchMock);

    await expect(ceremonyHeader("money_kill_run", {})).rejects.toThrow(/WebAuthn is not available/);
    expect(fetchMock).not.toHaveBeenCalled();
  });
});

// ---------------------------------------------------------------------------
// invokeWithCeremony: the 428-retry-once path (mocked transport), and the
// probe-already-required optimistic path
// ---------------------------------------------------------------------------

describe("invokeWithCeremony", () => {
  it("dispatches plainly and never touches WebAuthn at all when no passkey is enrolled (the trial fallback)", async () => {
    vi.mocked(isWebShell).mockReturnValue(true);
    const fetchMock = routedFetch({ "/webauthn/passkeys": { passkeys: [], webauthn_required: false } });
    vi.stubGlobal("fetch", fetchMock);
    vi.mocked(invokeBackend).mockResolvedValue({ summary: "ok" });

    await expect(invokeWithCeremony("money_kill_run", { run_id: "a", reason: "test" })).resolves.toEqual({
      summary: "ok",
    });
    expect(invokeBackend).toHaveBeenCalledTimes(1);
    expect(invokeBackend).toHaveBeenCalledWith("money_kill_run", { run_id: "a", reason: "test" });
    // only the passkeys probe, never action/start - no ceremony was run
    expect(fetchMock).toHaveBeenCalledTimes(1);
  });

  it("on a 428 'webauthn required' rejection, runs the ceremony once and retries exactly once, with the header", async () => {
    vi.mocked(isWebShell).mockReturnValue(true);
    vi.stubGlobal("window", secureWindow());
    vi.stubGlobal(
      "fetch",
      routedFetch({
        "/webauthn/passkeys": { passkeys: [], webauthn_required: false },
        "/webauthn/action/start": {
          challenge: b64urlEncode(new Uint8Array([1, 2]).buffer),
          rp_id: "localhost",
          timeout: 120000,
          user_verification: "preferred",
          allow_credentials: [{ type: "public-key", id: b64urlEncode(new Uint8Array([9]).buffer) }],
        },
      }),
    );
    vi.stubGlobal("navigator", {
      credentials: {
        get: vi.fn(async () => ({
          rawId: new Uint8Array([9]).buffer,
          response: {
            clientDataJSON: new TextEncoder().encode("{}").buffer,
            authenticatorData: new Uint8Array([1]).buffer,
            signature: new Uint8Array([2]).buffer,
          },
        })),
      },
    });
    vi.mocked(invokeBackend)
      .mockRejectedValueOnce({ error: "a webauthn assertion is required for this command", webauthn: "required" })
      .mockResolvedValueOnce({ summary: "killed" });

    await expect(invokeWithCeremony("money_kill_run", { run_id: "a" })).resolves.toEqual({ summary: "killed" });

    expect(invokeBackend).toHaveBeenCalledTimes(2);
    expect(invokeBackend).toHaveBeenNthCalledWith(1, "money_kill_run", { run_id: "a" });
    const retryCall = vi.mocked(invokeBackend).mock.calls[1]!;
    expect(retryCall[0]).toBe("money_kill_run");
    expect(retryCall[1]).toEqual({ run_id: "a" });
    expect(typeof retryCall[2]?.["x-genaryx-webauthn"]).toBe("string");
  });

  it("never retries twice - a non-428 rejection from the retry itself propagates, coerced to a plain Error", async () => {
    vi.mocked(isWebShell).mockReturnValue(true);
    vi.stubGlobal("window", secureWindow());
    vi.stubGlobal(
      "fetch",
      routedFetch({
        "/webauthn/passkeys": { passkeys: [], webauthn_required: false },
        "/webauthn/action/start": {
          challenge: b64urlEncode(new Uint8Array([1]).buffer),
          rp_id: "localhost",
          timeout: 1,
          user_verification: "preferred",
          allow_credentials: [],
        },
      }),
    );
    vi.stubGlobal("navigator", {
      credentials: {
        get: vi.fn(async () => ({
          rawId: new Uint8Array([1]).buffer,
          response: {
            clientDataJSON: new Uint8Array([1]).buffer,
            authenticatorData: new Uint8Array([1]).buffer,
            signature: new Uint8Array([1]).buffer,
          },
        })),
      },
    });
    vi.mocked(invokeBackend)
      .mockRejectedValueOnce({ error: "a webauthn assertion is required for this command", webauthn: "required" })
      .mockRejectedValueOnce({ error: "webauthn: bad signature" });

    await expect(invokeWithCeremony("money_kill_run", {})).rejects.toThrow("webauthn: bad signature");
    expect(invokeBackend).toHaveBeenCalledTimes(2);
  });

  it("lets a genuine domain error (tagged with its own kind) pass through unchanged after a successful ceremony", async () => {
    vi.mocked(isWebShell).mockReturnValue(true);
    vi.stubGlobal("window", secureWindow());
    vi.stubGlobal(
      "fetch",
      routedFetch({
        "/webauthn/passkeys": {
          passkeys: [{ credential_id: "c1", label: "x", created_at: "2026-01-01T00:00:00Z" }],
          webauthn_required: true,
        },
        "/webauthn/action/start": {
          challenge: b64urlEncode(new Uint8Array([1]).buffer),
          rp_id: "localhost",
          timeout: 1,
          user_verification: "preferred",
          allow_credentials: [],
        },
      }),
    );
    vi.stubGlobal("navigator", {
      credentials: {
        get: vi.fn(async () => ({
          rawId: new Uint8Array([1]).buffer,
          response: {
            clientDataJSON: new Uint8Array([1]).buffer,
            authenticatorData: new Uint8Array([1]).buffer,
            signature: new Uint8Array([1]).buffer,
          },
        })),
      },
    });
    vi.mocked(invokeBackend).mockRejectedValueOnce({ kind: "cloud", status: 500, message: "boom" });

    await expect(invokeWithCeremony("money_kill_run", {})).rejects.toEqual({
      kind: "cloud",
      status: 500,
      message: "boom",
    });
  });

  it("runs the ceremony FIRST, with no plain attempt at all, when the cached probe already says it is required", async () => {
    vi.mocked(isWebShell).mockReturnValue(true);
    vi.stubGlobal("window", secureWindow());
    vi.stubGlobal(
      "fetch",
      routedFetch({
        "/webauthn/passkeys": {
          passkeys: [{ credential_id: "c1", label: "x", created_at: "2026-01-01T00:00:00Z" }],
          webauthn_required: true,
        },
        "/webauthn/action/start": {
          challenge: b64urlEncode(new Uint8Array([1]).buffer),
          rp_id: "localhost",
          timeout: 1,
          user_verification: "preferred",
          allow_credentials: [],
        },
      }),
    );
    vi.stubGlobal("navigator", {
      credentials: {
        get: vi.fn(async () => ({
          rawId: new Uint8Array([1]).buffer,
          response: {
            clientDataJSON: new Uint8Array([1]).buffer,
            authenticatorData: new Uint8Array([1]).buffer,
            signature: new Uint8Array([1]).buffer,
          },
        })),
      },
    });
    vi.mocked(invokeBackend).mockResolvedValueOnce({ summary: "ok" });

    await expect(invokeWithCeremony("money_kill_run", { run_id: "a" })).resolves.toEqual({ summary: "ok" });

    expect(invokeBackend).toHaveBeenCalledTimes(1);
    const call = vi.mocked(invokeBackend).mock.calls[0]!;
    expect(call[0]).toBe("money_kill_run");
    expect(call[1]).toEqual({ run_id: "a" });
    expect(typeof call[2]?.["x-genaryx-webauthn"]).toBe("string");
  });

  it("falls back to a plain dispatch when the probe itself fails, rather than blocking the command on it", async () => {
    vi.mocked(isWebShell).mockReturnValue(true);
    vi.stubGlobal("fetch", vi.fn().mockRejectedValue(new Error("network down")));
    vi.mocked(invokeBackend).mockResolvedValueOnce({ summary: "ok" });

    await expect(invokeWithCeremony("money_kill_run", { run_id: "a" })).resolves.toEqual({ summary: "ok" });
    expect(invokeBackend).toHaveBeenCalledWith("money_kill_run", { run_id: "a" });
  });
});
