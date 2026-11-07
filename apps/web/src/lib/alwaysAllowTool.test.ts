import { beforeEach, describe, expect, it, vi } from "vitest";

const ceremony = vi.fn(async () => ({ summary: "s" }));
const plain = vi.fn(async () => ({}));

vi.mock("./webauthn", () => ({ invokeWithCeremony: ceremony }));
vi.mock("./transport", () => ({
  hasBackend: () => true,
  invokeBackend: plain,
  requiredRoleFromCommandError: () => null,
}));

beforeEach(() => {
  ceremony.mockClear();
  plain.mockClear();
});

describe("the command behind the button", () => {
  it("always_allow_goes_through_the_same_ceremony_as_a_grant", async () => {
    const { alwaysAllowTool } = await import("./policy");
    await alwaysAllowTool("ap_1");
    expect(ceremony).toHaveBeenCalledOnce();
    expect(plain).not.toHaveBeenCalled();
  });

  it("the_browser_names_only_the_hold_and_never_the_agent_or_the_tool", async () => {
    const { alwaysAllowTool } = await import("./policy");
    await alwaysAllowTool("ap_1");
    expect(ceremony).toHaveBeenCalledWith("policy_grant_tool", { id: "ap_1" });
  });
});
