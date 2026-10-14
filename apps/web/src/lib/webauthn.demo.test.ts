import { afterEach, describe, expect, it, vi } from "vitest";

// Built the way the demo build is: the mock flag set before `webauthn.ts`
// loads, so `MOCK` is read as true there. Nothing here reaches a network or a
// passkey prompt, which is the point of the demo answer.
async function loadWebauthnInDemo() {
  vi.resetModules();
  vi.stubEnv("VITE_GENARYX_MOCK", "1");
  vi.stubEnv("VITE_GENARYX_API", "");
  return import("./webauthn");
}

afterEach(() => {
  vi.unstubAllEnvs();
});

describe("passkeys in the demo", () => {
  it("the_demo_lists_no_passkeys_and_asks_no_box", async () => {
    const { listPasskeys } = await loadWebauthnInDemo();

    const probe = await listPasskeys();

    expect(probe.passkeys).toEqual([]);
    expect(probe.webauthn_required).toBe(false);
  });

  it("the_demo_passkey_enrollment_says_nothing_was_done", async () => {
    const { enrollPasskey } = await loadWebauthnInDemo();

    await expect(enrollPasskey("laptop", "operator-password")).rejects.toThrow("nothing was done");
  });

  it("the_demo_passkey_removal_says_nothing_was_done", async () => {
    const { removePasskey } = await loadWebauthnInDemo();

    await expect(removePasskey("some-credential")).rejects.toThrow("nothing was done");
  });
});
