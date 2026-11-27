import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("./transport", () => ({
  hasBackend: vi.fn(),
  invokeBackend: vi.fn(),
  isWebShell: vi.fn(),
}));
vi.mock("./consoleState", () => ({ notifyConsoleStateChanged: vi.fn() }));

import { hasBackend, invokeBackend, isWebShell } from "./transport";
import { notifyConsoleStateChanged } from "./consoleState";
import {
  EDIT_NEEDS_STORE,
  blockAgent,
  canEditAgentRecord,
  fetchOrgDirectory,
  reassignAgentUnit,
  setAgentBehaviour,
  setAgentBudget,
  transferAgentOwner,
} from "./agentActions";

const box = vi.mocked(hasBackend);
const web = vi.mocked(isWebShell);
const ask = vi.mocked(invokeBackend);
const notify = vi.mocked(notifyConsoleStateChanged);

/** The mock preview: a backend that is not a web shell. */
const preview = () => {
  box.mockReturnValue(true);
  web.mockReturnValue(false);
};

beforeEach(() => {
  vi.resetAllMocks();
});

describe("where an agent record can be edited", () => {
  it("only_the_preview_edits_a_real_box_and_a_bare_build_do_not", () => {
    preview();
    expect(canEditAgentRecord()).toBe(true);
    web.mockReturnValue(true);
    expect(canEditAgentRecord()).toBe(false);
    box.mockReturnValue(false);
    web.mockReturnValue(false);
    expect(canEditAgentRecord()).toBe(false);
  });

  it("on_a_real_box_an_edit_sends_nothing_and_says_why", async () => {
    box.mockReturnValue(true);
    web.mockReturnValue(true);
    for (const run of [
      () => setAgentBudget("a", 1),
      () => reassignAgentUnit("a", "t"),
      () => transferAgentOwner("a", "o"),
      () => setAgentBehaviour("a", []),
    ]) {
      await expect(run()).resolves.toEqual({ ok: false, reason: EDIT_NEEDS_STORE });
    }
    expect(ask).not.toHaveBeenCalled();
  });
});

describe("an edit in the preview", () => {
  it.each([
    ["agent_set_budget", () => setAgentBudget("a", 2.5), { agent_id: "a", budget_usd: 2.5 }],
    ["agent_reassign_unit", () => reassignAgentUnit("a", "t"), { agent_id: "a", team: "t" }],
    ["agent_transfer_owner", () => transferAgentOwner("a", "o"), { agent_id: "a", owner: "o" }],
    ["agent_set_behaviour", () => setAgentBehaviour("a", ["x"]), { agent_id: "a", allowed: ["x"] }],
  ] as const)("%s_sends_its_command_and_returns_the_record", async (command, run, args) => {
    preview();
    ask.mockResolvedValue({ id: "a" } as never);
    await expect(run()).resolves.toEqual({ ok: true, record: { id: "a" } });
    expect(ask).toHaveBeenCalledWith(command, args);
  });

  it.each([
    ["a preview-only refusal", { error: "preview_only", command: "agent_set_budget" }, EDIT_NEEDS_STORE],
    ["a plain sentence", "no such agent", "no such agent"],
    ["an error field", { error: "locked" }, "locked"],
    ["a message field", { message: "busy" }, "busy"],
    ["an empty sentence", "", "the change did not land"],
    ["an object with no text", { error: 3 }, "the change did not land"],
    ["null", null, "the change did not land"],
  ])("a_failure_that_is_%s_is_printed_as_its_reason", async (_n, err, reason) => {
    preview();
    ask.mockRejectedValue(err);
    await expect(setAgentBudget("a", 1)).resolves.toEqual({ ok: false, reason });
  });
});

describe("the org directory", () => {
  it("is_null_where_nothing_is_editable_and_when_the_read_fails", async () => {
    box.mockReturnValue(false);
    await expect(fetchOrgDirectory()).resolves.toBeNull();
    preview();
    ask.mockRejectedValue(new Error("x"));
    await expect(fetchOrgDirectory()).resolves.toBeNull();
  });

  it("is_what_the_preview_answers", async () => {
    preview();
    const dir = { teams: [], users: [] };
    ask.mockResolvedValue(dir as never);
    await expect(fetchOrgDirectory()).resolves.toEqual(dir);
  });
});

describe("freezing one agent", () => {
  it("sends_the_block_and_tells_every_open_panel_to_re_read", async () => {
    box.mockReturnValue(true);
    ask.mockResolvedValue(null as never);
    await expect(blockAgent("a", true)).resolves.toBeNull();
    expect(ask).toHaveBeenCalledWith("agent_block", { agent_id: "a", blocked: true });
    expect(notify).toHaveBeenCalledTimes(1);
  });

  it("with_no_backend_nothing_is_sent_and_a_failure_is_swallowed_into_null", async () => {
    box.mockReturnValue(false);
    await expect(blockAgent("a", false)).resolves.toBeNull();
    expect(ask).not.toHaveBeenCalled();
    box.mockReturnValue(true);
    ask.mockRejectedValue(new Error("down"));
    await expect(blockAgent("a", false)).resolves.toBeNull();
  });
});
