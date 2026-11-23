/**
 * Invariant 50: a command only the preview answers never leaves the browser on
 * a real box. The box has no dispatch arm for it, so the request could only
 * come back 403 or 404, which every caller swallowed into `null`.
 */

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const fetched: string[] = [];

beforeEach(() => {
  fetched.length = 0;
  vi.resetModules();
  vi.stubEnv("VITE_GENARYX_API", "/api");
  vi.stubEnv("VITE_GENARYX_MOCK", "");
  vi.stubGlobal("fetch", async (url: string) => {
    fetched.push(String(url));
    return new Response("[]", { status: 200 });
  });
});

afterEach(() => {
  vi.unstubAllEnvs();
  vi.unstubAllGlobals();
});

describe("a real box and the preview-only commands", () => {
  it("a_real_box_refuses_every_preview_only_command_before_any_request", async () => {
    const { invokeBackend } = await import("./transport");
    const { PREVIEW_ONLY_COMMANDS } = await import("./previewOnly");
    expect(PREVIEW_ONLY_COMMANDS.length).toBeGreaterThan(0);
    for (const command of PREVIEW_ONLY_COMMANDS) {
      await expect(invokeBackend(command, {})).rejects.toEqual({ error: "preview_only", command });
    }
    expect(fetched).toEqual([]);
  });

  it("a_real_box_still_sends_a_command_it_has_an_arm_for", async () => {
    const { invokeBackend } = await import("./transport");
    await expect(invokeBackend("money_runs")).resolves.toEqual([]);
    expect(fetched).toEqual(["/api/command/money_runs"]);
  });

  it("the_org_directory_which_no_box_answers_is_preview_only", async () => {
    const { isPreviewOnly } = await import("./previewOnly");
    expect(isPreviewOnly("org_directory")).toBe(true);
    expect(isPreviewOnly("money_runs")).toBe(false);
  });
});
