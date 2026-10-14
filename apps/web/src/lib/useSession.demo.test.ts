import { afterEach, describe, expect, it, vi } from "vitest";

// `useSession` is a React hook. Under Node there is no renderer, so the two
// hooks it uses are replaced with plain functions: `useState` hands back its
// initial value, and `useEffect` records its effect so the test can run it
// by hand. That is enough to read what the hook returns on first render and
// what it asks the box for.
const hooks = vi.hoisted(() => ({
  effects: [] as Array<() => void | (() => void)>,
  fetchSession: vi.fn(async () => ({
    configured: true,
    signed_in: true,
    user: "box-operator",
    role: "viewer" as const,
    method: "local" as const,
    oidc_available: false,
  })),
}));

vi.mock("react", () => ({
  useState: (init: unknown) => [typeof init === "function" ? (init as () => unknown)() : init, () => {}],
  useEffect: (effect: () => void | (() => void)) => {
    hooks.effects.push(effect);
  },
}));

vi.mock("./session", () => ({ fetchSession: hooks.fetchSession }));

afterEach(() => {
  vi.unstubAllEnvs();
  hooks.effects.length = 0;
  hooks.fetchSession.mockClear();
});

/** Load a fresh copy of the hook under the given build flags, call it once as
 * a first render would, then run whatever effects it registered. */
async function firstRender(mock: boolean, webApi: string) {
  vi.resetModules();
  vi.stubEnv("VITE_GENARYX_MOCK", mock ? "1" : "");
  vi.stubEnv("VITE_GENARYX_API", webApi);
  const { useSession } = await import("./useSession");
  const initial = useSession();
  for (const effect of hooks.effects.splice(0)) {
    await effect();
  }
  return initial;
}

describe("useSession", () => {
  it("the_web_build_still_asks_the_box_for_the_session", async () => {
    const initial = await firstRender(false, "/api");

    expect(initial).toBeNull();
    expect(hooks.fetchSession).toHaveBeenCalledTimes(1);
  });

  it("the_demo_build_answers_a_signed_in_admin_without_asking_a_box", async () => {
    const initial = await firstRender(true, "");

    expect(initial).toMatchObject({ signed_in: true, role: "admin", user: "ops" });
    expect(hooks.fetchSession).not.toHaveBeenCalled();
  });

  it("a_bare_preview_has_no_session_and_asks_nothing", async () => {
    const initial = await firstRender(false, "");

    expect(initial).toBeNull();
    expect(hooks.fetchSession).not.toHaveBeenCalled();
  });
});
