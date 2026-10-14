import { afterEach, describe, expect, it, vi } from "vitest";

afterEach(() => {
  vi.unstubAllEnvs();
});

/** Load the connect card's planner under the given build flags. */
async function loadPlanner(mock: boolean) {
  vi.resetModules();
  vi.stubEnv("VITE_GENARYX_MOCK", mock ? "1" : "");
  vi.stubEnv("VITE_GENARYX_API", mock ? "" : "/api");
  return import("./FelyxConnectCard");
}

describe("the Felyx connect planner", () => {
  it("the_demo_felyx_connect_answers_without_an_api_key", async () => {
    const { PROVIDERS, planConnect } = await loadPlanner(true);
    const cloud = PROVIDERS.find((p) => !p.local);
    if (!cloud) throw new Error("no cloud provider to test");

    const plan = planConnect(
      { provider: cloud, model: cloud.defaultModel, baseUrl: "", apiKey: "", maxUsd: "5" },
      { supported: true },
    );

    expect(plan.send).toBe(true);
  });

  it("the_real_felyx_connect_still_needs_an_api_key", async () => {
    const { PROVIDERS, planConnect } = await loadPlanner(false);
    const cloud = PROVIDERS.find((p) => !p.local);
    if (!cloud) throw new Error("no cloud provider to test");

    const plan = planConnect(
      { provider: cloud, model: cloud.defaultModel, baseUrl: "", apiKey: "", maxUsd: "5" },
      { supported: true },
    );

    expect(plan.send).toBe(false);
  });
});
