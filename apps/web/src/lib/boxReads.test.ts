/**
 * The reads whose failure is an answer, not an exception: the Keys, Record and
 * Notifications panels show WHY the box could not answer (its own error text,
 * or that it did not answer at all) instead of an empty panel; the graph reads
 * render an empty graph rather than trap; Felyx's status says why it is off;
 * and a proposal the operator approved is journaled or says why not.
 */

import { beforeEach, describe, expect, it, vi } from "vitest";

const box = vi.hoisted(() => ({
  present: true,
  calls: [] as { command: string; args: unknown }[],
  answer: (_command: string): unknown => ({ ok: true }),
}));

vi.mock("./transport", () => ({
  hasBackend: () => box.present,
  isWebShell: () => box.present,
  webApiBase: () => "",
  subscribeBackend: async () => () => {},
  requiredRoleFromCommandError: () => null,
  invokeBackend: async (command: string, args?: unknown) => {
    box.calls.push({ command, args });
    const a = box.answer(command);
    if (a && typeof a === "object" && "__reject" in a) throw (a as { __reject: unknown }).__reject;
    return a;
  },
}));

import { fetchKeys } from "./keyCustody";
import { fetchRecord, verifyRecord } from "./trailRecord";
import { chainSentence, fetchNotifications } from "./sentJournal";
import { fetchAgentEvents, fetchAgentGraph, fetchAgentSlice, isUserId, shortAgentLabel } from "./graph";
import { askCopilot, describeCopilotError, explainIncident, fetchCopilotStatus, logProposalApproved } from "./copilot";
import { fetchAgentRecord, teamOf, userHandle } from "./agentRecord";
import { formatQualityMean } from "./quality";
import { lifecyclePillClass } from "./lifecycleTypes";

const fail = (value: unknown) => () => ({ __reject: value });

beforeEach(() => {
  box.present = true;
  box.calls = [];
  box.answer = () => ({ ok: true });
  vi.spyOn(console, "error").mockImplementation(() => {});
});

describe("Keys, Record and Notifications say why the box did not answer", () => {
  const reads: [string, () => Promise<unknown>, string][] = [
    ["keys", fetchKeys, "keys_custody"],
    ["record", fetchRecord, "record_status"],
    ["record verify", verifyRecord, "record_verify"],
    ["notifications", fetchNotifications, "notifications_sent"],
  ];

  it.each(reads)("%s: a panel the box answered is handed on", async (_l, read, command) => {
    box.answer = () => ({ panel: command });
    expect(await read()).toEqual({ ok: true, panel: { panel: command } });
    expect(box.calls).toEqual([{ command, args: undefined }]);
  });

  it.each(reads)("%s: the box's own error text is the reason", async (_l, read) => {
    box.answer = fail({ error: "record not configured on this box" });
    expect(await read()).toEqual({ ok: false, reason: "record not configured on this box" });
    box.answer = fail("plain refusal");
    expect(await read()).toEqual({ ok: false, reason: "plain refusal" });
  });

  it.each(reads)("%s: anything else is 'the box did not answer'", async (_l, read) => {
    for (const err of [new Error("x"), { error: 500 }, null, 42]) {
      box.answer = fail(err);
      expect(await read()).toEqual({ ok: false, reason: "the box did not answer" });
    }
  });

  it.each(reads)("%s: with no box there is no claim at all", async (_l, read) => {
    box.present = false;
    expect(await read()).toBeNull();
    expect(box.calls).toEqual([]);
  });
});

describe("the notifications chain sentence", () => {
  it("names how many records the chain binds, and which could not be checked", () => {
    expect(chainSentence({ state: "verifies", chained: 12, heads: 1, unverifiable: 0 } as never, false)).toBe(
      "chain verifies: 12 chained, 1 head(s)",
    );
    expect(chainSentence({ state: "verifies", chained: 12, heads: 2, unverifiable: 3 } as never, true)).toBe(
      "chain verifies: 12 chained, 2 head(s), 3 could not be checked (judged over the newest part of the journal only)",
    );
  });
  it("says where a broken chain breaks, and that one record binds nothing", () => {
    expect(chainSentence({ state: "broken", first_line: 40, breaks: 2 } as never, false)).toMatch(
      /^chain BROKEN at line 40: 2 record\(s\) name a predecessor/,
    );
    expect(chainSentence({ state: "nothing_bound" } as never, true)).toBe(
      "chain: one record or none, which binds nothing, so it is not called verified",
    );
  });
});

describe("the graph reads never trap", () => {
  it("ask for the agent's slice and events by id", async () => {
    box.answer = (c) => (c === "agent_graph" ? { nodes: [1], edges: [], width: 5, height: 5 } : c === "agent_slice" ? { node: 1 } : [{ id: 1 }]);
    expect(await fetchAgentGraph()).toEqual({ nodes: [1], edges: [], width: 5, height: 5 });
    expect(await fetchAgentSlice("agent://a/b")).toEqual({ node: 1 });
    expect(await fetchAgentEvents("agent://a/b", 50)).toEqual([{ id: 1 }]);
    expect(box.calls).toEqual([
      { command: "agent_graph", args: undefined },
      { command: "agent_slice", args: { agent_id: "agent://a/b" } },
      { command: "agent_events", args: { agent_id: "agent://a/b", limit: 50 } },
    ]);
  });

  it("render empty on a failure or with no box", async () => {
    const empty = { nodes: [], edges: [], width: 1000, height: 1000 };
    const emptySlice = { node: null, parents: [], children: [] };
    box.answer = fail(new Error("down"));
    expect(await fetchAgentGraph()).toEqual(empty);
    expect(await fetchAgentSlice("a")).toEqual(emptySlice);
    expect(await fetchAgentEvents("a", 1)).toEqual([]);
    box.present = false;
    expect(await fetchAgentGraph()).toEqual(empty);
    expect(await fetchAgentSlice("a")).toEqual(emptySlice);
    expect(await fetchAgentEvents("a", 1)).toEqual([]);
  });

  it("label a subject by its path, and tell a person from an agent", () => {
    expect(shortAgentLabel("agent://acme.example/finops/forecaster")).toBe("finops/forecaster");
    expect(shortAgentLabel("user://acme.example/ops")).toBe("ops");
    expect(shortAgentLabel("key:forge-imposter")).toBe("key:forge-imposter");
    expect(isUserId("user://acme/ops")).toBe(true);
    expect(isUserId("agent://acme/ops")).toBe(false);
  });
});

describe("Felyx", () => {
  it("is off with the reason when the box fails, or there is none", async () => {
    box.answer = fail(new Error("provider not configured"));
    expect(await fetchCopilotStatus()).toMatchObject({ enabled: false, disabled_reason: "provider not configured" });
    box.answer = fail("refused");
    expect(await fetchCopilotStatus()).toMatchObject({ enabled: false, disabled_reason: "refused" });
    box.present = false;
    expect(await fetchCopilotStatus()).toMatchObject({ enabled: false, disabled_reason: "No backend to talk to." });
  });

  it("asks and explains through their own commands, and refuses with no box", async () => {
    box.answer = () => ({ text: "answer" });
    expect(await askCopilot("which run?")).toEqual({ text: "answer" });
    expect(await explainIncident("bus:7")).toEqual({ text: "answer" });
    expect(await fetchCopilotStatus()).toEqual({ text: "answer" });
    expect(box.calls.map((c) => [c.command, c.args])).toEqual([
      ["copilot_ask", { question: "which run?" }],
      ["copilot_explain", { incident_id: "bus:7" }],
      ["copilot_status", undefined],
    ]);
    box.present = false;
    await expect(askCopilot("q")).rejects.toThrow("No backend to talk to.");
    await expect(explainIncident("i")).rejects.toThrow("No backend to talk to.");
  });

  it("says an error as text, whatever shape it came in", () => {
    expect(describeCopilotError("plain")).toBe("plain");
    expect(describeCopilotError(new Error("boom"))).toBe("boom");
    expect(describeCopilotError(404)).toBe("404");
  });

  it("journals an approved proposal, or says why it could not", async () => {
    box.answer = () => ({ journaled: true, journal_error: null });
    expect(await logProposalApproved("kill_run", "run-1", { reason: "x" })).toEqual({ journaled: true, journal_error: null });
    expect(box.calls[0]).toEqual({ command: "copilot_log_proposal_approved", args: { kind: "kill_run", target: "run-1", params: { reason: "x" } } });
    box.answer = fail(new Error("chain closed"));
    expect(await logProposalApproved("k", "t", null)).toEqual({ journaled: false, journal_error: "chain closed" });
    box.answer = fail("no");
    expect(await logProposalApproved("k", "t", null)).toEqual({ journaled: false, journal_error: "no" });
    box.present = false;
    expect(await logProposalApproved("k", "t", null)).toEqual({ journaled: false, journal_error: "No backend to talk to." });
  });
});

describe("small readings the cards print", () => {
  it("an agent record a box does not keep is no record, not an error", async () => {
    box.answer = fail({ error: "unknown command" });
    expect(await fetchAgentRecord("agent://a/b")).toBeNull();
    box.present = false;
    expect(await fetchAgentRecord("agent://a/b")).toBeNull();
  });

  it("an agent's team is its first path segment, a user's handle its last", () => {
    expect(teamOf("agent://acme.example/finops/forecaster")).toBe("finops");
    expect(teamOf("agent://acme.example/solo")).toBe("");
    expect(userHandle("user://acme.example/people/ops")).toBe("ops");
    expect(userHandle("ops")).toBe("ops");
  });

  it("a quality mean says unmeasured, and names what went unanswered only when it can", () => {
    const base = { mean_score: 0.8312, case_count: 40, unanswered_count: 0, unanswered_by_reason: [], unanswered_supported: true };
    expect(formatQualityMean({ ...base, mean_score: null })).toBe("unmeasured");
    expect(formatQualityMean(base)).toBe("mean 0.83 over 40 answered");
    expect(
      formatQualityMean({
        ...base,
        unanswered_count: 3,
        unanswered_by_reason: [
          { reason: "timeout", count: 2 },
          { reason: "refused", count: 1 },
        ],
      }),
    ).toBe("mean 0.83 over 40 answered, 3 unanswered (timeout: 2, refused: 1)");
    expect(formatQualityMean({ ...base, unanswered_count: 3, unanswered_supported: false })).toBe("mean 0.83 over 40 answered");
  });

  it("a lifecycle state picks its pill, and a live run has none of its own", () => {
    expect(lifecyclePillClass("killed")).toBe("dead");
    expect(lifecyclePillClass("stopped")).toBe("stopped");
    expect(lifecyclePillClass("frozen")).toBe("frozen");
    expect(lifecyclePillClass("live")).toBeNull();
  });
});
