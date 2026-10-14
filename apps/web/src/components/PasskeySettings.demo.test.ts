import { afterEach, describe, expect, it, vi } from "vitest";

// The Passkeys panel is driven by React hooks. Under Node there is no
// renderer, so `useState` hands back queued values in the panel's own order
// (passkeys, policyRequires, label, enrolling, removing, password,
// needPassword, error) and records every setter call, and `useEffect` does
// nothing. The panel's returned element tree is then read directly.
const hooks = vi.hoisted(() => ({
  queue: [] as unknown[],
  sets: [] as unknown[],
}));

vi.mock("react", async (importOriginal) => {
  const actual = await importOriginal<typeof import("react")>();
  return {
    ...actual,
    useState: (init: unknown) => {
      const value =
        hooks.queue.length > 0 ? hooks.queue.shift() : typeof init === "function" ? (init as () => unknown)() : init;
      return [
        value,
        (next: unknown) => {
          hooks.sets.push(next);
        },
      ];
    },
    useEffect: () => {},
  };
});

/** A first enrolment: no passkey yet, so the operator password field is shown. */
const FIRST_ENROLMENT = [[], false, "", false, null, "", false, null];

afterEach(() => {
  vi.unstubAllEnvs();
  vi.unstubAllGlobals();
  hooks.queue.length = 0;
  hooks.sets.length = 0;
});

type El = { type: unknown; props: Record<string, unknown> };

function elements(node: unknown, out: El[] = []): El[] {
  if (Array.isArray(node)) {
    for (const child of node) elements(child, out);
  } else if (node && typeof node === "object" && "props" in node) {
    const el = node as El;
    out.push(el);
    elements(el.props.children, out);
  }
  return out;
}

/** Build the panel under the given build flags. The enrolment controls only
 * render in a secure context with WebAuthn, as on localhost, so one is stubbed. */
async function renderPasskeys(mock: boolean) {
  vi.resetModules();
  vi.stubEnv("VITE_GENARYX_MOCK", mock ? "1" : "");
  vi.stubEnv("VITE_GENARYX_API", mock ? "" : "/api");
  vi.stubGlobal("window", { isSecureContext: true, PublicKeyCredential: function PublicKeyCredential() {} });
  hooks.queue.push(...FIRST_ENROLMENT);
  const { PasskeySettings } = await import("./PasskeySettings");
  return PasskeySettings();
}

const operatorPassword = (tree: unknown) =>
  elements(tree).find((el) => el.type === "input" && el.props.type === "password");
const addPasskey = (tree: unknown) =>
  elements(tree).find((el) => el.type === "button" && el.props.children === "Add passkey");

describe("the Passkeys panel", () => {
  it("the_demo_passkey_operator_password_is_disabled", async () => {
    const input = operatorPassword(await renderPasskeys(true));

    expect(input).toBeDefined();
    expect(input?.props.disabled).toBe(true);
    expect(input?.props.placeholder).toBe("disabled in the demo");
  });

  it("the_demo_add_passkey_still_answers_with_the_simulated_result", async () => {
    const button = addPasskey(await renderPasskeys(true));

    expect(button?.props.disabled).toBeFalsy();
    (button?.props.onClick as () => void)();
    // enrolPasskey settles on a microtask; one macrotask lets it finish.
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(hooks.sets.some((v) => typeof v === "string" && v.includes("nothing was done"))).toBe(true);
  });

  it("the_real_passkey_operator_password_stays_enabled", async () => {
    const input = operatorPassword(await renderPasskeys(false));

    expect(input).toBeDefined();
    expect(input?.props.disabled).toBe(false);
    expect(input?.props.placeholder).toBe("operator password");
  });
});
