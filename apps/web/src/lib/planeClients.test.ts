// @vitest-environment jsdom
/**
 * The plane clients (`lib/money.ts`, `lib/policy.ts`, `lib/remote.ts`, ...) are
 * the console's only way to ask a plane anything, and they share a contract:
 *
 * - each call is one named command with its arguments in the box's own
 *   spelling (snake_case, an empty optional field sent as null, not "");
 * - a sensitive command goes through the passkey ceremony, never the plain
 *   transport, so a refactor cannot quietly drop the ceremony from a kill;
 * - a refusal is normalised once: the role gate's 403 becomes `role_required`,
 *   a plane's own structured error passes through unchanged, anything else
 *   becomes the plane's catch-all kind carrying the message;
 * - with no box at all a call refuses with the plane's "no environment" error
 *   and never reaches the transport;
 * - a read that is allowed to say "nothing" answers null or empty, not a
 *   fabricated row, when the box fails.
 *
 * Only the transport and the ceremony are faked here.
 */

import { beforeEach, describe, expect, it, vi } from "vitest";

const box = vi.hoisted(() => ({
  present: true,
  calls: [] as { via: "plain" | "ceremony"; command: string; args: unknown }[],
  answer: (_command: string): unknown => ({ ok: true }),
}));

vi.mock("./transport", async () => {
  const real = await vi.importActual<typeof import("./transport")>("./transport");
  return {
    hasBackend: () => box.present,
    isWebShell: () => box.present,
    webApiBase: () => "/api",
    subscribeBackend: async () => () => {},
    requiredRoleFromCommandError: real.requiredRoleFromCommandError,
    invokeBackend: async (command: string, args?: unknown) => {
      box.calls.push({ via: "plain", command, args });
      const a = box.answer(command);
      if (a instanceof Rejection) throw a.value;
      return a;
    },
  };
});

vi.mock("./webauthn", () => ({
  invokeWithCeremony: async (command: string, args?: unknown) => {
    box.calls.push({ via: "ceremony", command, args });
    const a = box.answer(command);
    if (a instanceof Rejection) throw a.value;
    return a;
  },
}));

class Rejection {
  constructor(public value: unknown) {}
}
const reject = (value: unknown) => () => new Rejection(value);

import * as money from "./money";
import * as policy from "./policy";
import * as remote from "./remote";
import * as identity from "./identity";
import * as memory from "./memory";
import * as quality from "./quality";
import * as crypto from "./crypto";
import * as drills from "./drills";
import * as evidence from "./evidence";
import * as admission from "./admission";
import * as onboard from "./onboard";
import * as delegation from "./delegation";
import * as entityRecords from "./entityRecords";
import * as agentProfile from "./agentProfile";
import * as agentStops from "./agentStops";
import * as quarantine from "./quarantine";
import * as replay from "./replay";
import * as egress from "./egress";
import * as busStatus from "./busStatus";
import { CONSOLE_STATE_CHANGED_EVENT } from "./consoleState";

beforeEach(() => {
  box.present = true;
  box.calls = [];
  box.answer = () => ({ ok: true });
  vi.spyOn(console, "error").mockImplementation(() => {});
});

type Route = [label: string, run: () => Promise<unknown>, via: "plain" | "ceremony", command: string, args: unknown];

const ROUTES: Route[] = [
  ["money overview", () => money.fetchOverview(), "plain", "money_overview", undefined],
  ["money runs", () => money.fetchRuns(), "plain", "money_runs", undefined],
  ["money spend window", () => money.fetchSpendWindow(30), "plain", "money_spend_window", { days: 30 }],
  ["money incidents", () => money.fetchIncidents(), "plain", "money_incidents", undefined],
  ["money savings", () => money.fetchSavings(), "plain", "money_savings", undefined],
  ["money owners", () => money.fetchOwners(), "plain", "money_owners", undefined],
  ["money gateways", () => money.fetchGateways(), "plain", "money_gateways", undefined],
  ["money unit caps", () => money.fetchUnitCaps(), "plain", "money_units", undefined],
  ["money ack", () => money.ackIncident("inc-1"), "plain", "money_ack_incident", { id: "inc-1" }],
  ["kill a run", () => money.killRun("run-1", "runaway"), "ceremony", "money_kill_run", { run_id: "run-1", reason: "runaway" }],
  ["set a run budget", () => money.setBudget("run-1", 2.5, "cap"), "ceremony", "money_set_budget", { run_id: "run-1", budget_usd: 2.5, reason: "cap" }],
  ["set a unit cap", () => money.setUnitBudget("treasury", 100, "month"), "ceremony", "money_set_unit_budget", { unit: "treasury", budget_usd: 100, reason: "month" }],
  ["policy approvals", () => policy.fetchApprovals(), "plain", "policy_list_approvals", undefined],
  ["policy list", () => policy.fetchPolicies(), "plain", "policy_list_policies", undefined],
  ["policy enforcement", () => policy.fetchEnforcementStatus(), "plain", "policy_enforcement_status", undefined],
  ["approval preview", () => policy.fetchApprovalPreview("ap-1"), "plain", "policy_approval_preview", { id: "ap-1" }],
  ["decide an approval", () => policy.decideApproval("ap-1", "approve" as never), "ceremony", "policy_decide_approval", { id: "ap-1", decision: "approve", seen_pvd: null }],
  ["decide having seen a preview", () => policy.decideApproval("ap-1", "deny" as never, "sha256:ab"), "ceremony", "policy_decide_approval", { id: "ap-1", decision: "deny", seen_pvd: "sha256:ab" }],
  ["always allow a tool", () => policy.alwaysAllowTool("ap-2"), "ceremony", "policy_grant_tool", { id: "ap-2" }],
  ["remote environment", () => remote.setRemoteEnvironment({ host: "h" } as never), "plain", "remote_set_environment", { request: { host: "h" } }],
  ["hetzner list, selector", () => remote.listHetznerServers("tok", "env=prod"), "plain", "remote_hetzner_list", { token: "tok", label_selector: "env=prod" }],
  ["hetzner list, blank selector", () => remote.listHetznerServers("tok", "   "), "plain", "remote_hetzner_list", { token: "tok", label_selector: null }],
  ["cloud list", () => remote.listCloudServers("aws" as never, { region: "eu" } as never), "plain", "remote_cloud_list", { provider: "aws", options: { region: "eu" } }],
  ["tunnel up", () => remote.connectTunnel(), "plain", "remote_wg_connect", undefined],
  ["tunnel down", () => remote.disconnectTunnel(), "plain", "remote_wg_disconnect", undefined],
  ["ssh check", () => remote.checkSshReachable(), "plain", "remote_ssh_check_reachable", undefined],
  ["read a file", () => remote.readRemoteFile("/var/log/x"), "plain", "remote_ssh_read_file", { path: "/var/log/x" }],
  ["start a tail", () => remote.startRemoteTail("/var/log/x", 42), "plain", "remote_ssh_tail_start", { path: "/var/log/x", from_offset: 42 }],
  ["stop a tail", () => remote.stopRemoteTail(), "plain", "remote_ssh_tail_stop", undefined],
  ["issue a wg peer", () => remote.issueOperatorWgConfig(), "ceremony", "remote_operator_wg_config", undefined],
  ["list wg peers", () => remote.listOperatorWgPeers(), "plain", "remote_operator_wg_peers", undefined],
  ["revoke a wg peer", () => remote.revokeOperatorWgPeer("pk="), "ceremony", "remote_operator_wg_revoke", { public_key: "pk=" }],
  ["identities", () => identity.fetchIdentities(), "plain", "identity_list_identities", undefined],
  ["identity alerts", () => identity.fetchAlerts(), "plain", "identity_list_alerts", undefined],
  ["remediations", () => identity.fetchRemediations(), "plain", "identity_list_remediations", undefined],
  ["rescan", () => identity.rescan(), "plain", "identity_rescan", undefined],
  ["memory stats, agent", () => memory.fetchStats("agent://a/b"), "plain", "memory_stats", { agent_id: "agent://a/b" }],
  ["memory stats, all", () => memory.fetchStats(" "), "plain", "memory_stats", { agent_id: null }],
  ["recall", () => memory.recall("q", 5, "hybrid" as never, ""), "plain", "memory_recall", { query: "q", limit: 5, mode: "hybrid", agent_id: null }],
  ["recall for an agent", () => memory.recall("q", 5, "fts" as never, "agent://a/b"), "plain", "memory_recall", { query: "q", limit: 5, mode: "fts", agent_id: "agent://a/b" }],
  ["why", () => memory.fetchWhy("m1"), "plain", "memory_why", { memory_id: "m1" }],
  ["forget", () => memory.forget("m1"), "plain", "memory_forget", { memory_id: "m1" }],
  ["quality runs", () => quality.fetchRunSummaries(), "plain", "quality_list_run_summaries", undefined],
  ["quality scores", () => quality.fetchRunScores("r1"), "plain", "quality_run_scores", { run_id: "r1" }],
  ["quality baselines", () => quality.fetchBaselines(), "plain", "quality_list_baselines", undefined],
  ["ncsc scan", () => crypto.scanNcsc("/srv"), "plain", "crypto_scan_ncsc", { path: "/srv" }],
  ["cbom scan", () => crypto.scanCbom("/srv"), "plain", "crypto_scan_cbom", { path: "/srv" }],
  ["evidence scan", () => crypto.scanEvidence("/srv"), "plain", "crypto_scan_evidence", { path: "/srv", sign_key: null }],
  ["verify evidence", () => crypto.verifyEvidence("/e.json"), "plain", "crypto_verify_evidence", { file: "/e.json" }],
  ["drills, blanks are null", () => drills.runDrills("/s", " ", false, ""), "plain", "drills_run", { scenario_dir: "/s", api_key: null, fail_on_skip: false, save_path: null }],
  ["drills, all set", () => drills.runDrills("/s", "k", true, "/out.json"), "plain", "drills_run", { scenario_dir: "/s", api_key: "k", fail_on_skip: true, save_path: "/out.json" }],
  ["evidence build", () => evidence.buildEvidence({ x: 1 } as never), "plain", "evidence_build", { request: { x: 1 } }],
  ["admission check", () => admission.runAdmissionCheck("k1", "agent://a/b"), "plain", "admission_check", { key_id: "k1", agent_id: "agent://a/b" }],
  ["admission baseline", () => admission.runAdmissionBaseline("/e", "m", "agent://a/b", "sk"), "plain", "admission_baseline", { evalset_path: "/e", model: "m", agent_id: "agent://a/b", api_key: "sk" }],
  ["onboard status, default", () => onboard.fetchOnboardStatus(), "plain", "onboard_status", { request: { map_path: null, passports_dir: null } }],
  ["onboard generate", () => onboard.generateOnboardBundle({ g: 1 } as never), "plain", "onboard_generate", { request: { g: 1 } }],
  ["onboard write", () => onboard.writeOnboardPassport({ w: 1 } as never), "plain", "onboard_write_passport", { request: { w: 1 } }],
  ["revoke a delegation", () => delegation.revokeDelegation("agent://a/b", "leaver"), "ceremony", "delegation_revoke", { subject: "agent://a/b", reason: "leaver" }],
  ["egress", () => { box.answer = () => ({ measured: true, totals: { fetched: 0, navigation_only: 0 } }); return egress.fetchEgress(); }, "plain", "egress_recent", { limit: 100 }],
];

describe("each call reaches the box as its own command, through the right door", () => {
  it.each(ROUTES)("%s", async (_label, run, via, command, args) => {
    await run();
    expect(box.calls).toEqual([{ via, command, args }]);
  });
});

describe("with no box at all nothing reaches the transport", () => {
  const cases: [string, () => Promise<unknown>, unknown][] = [
    ["money", () => money.fetchRuns(), { kind: "no_environment" }],
    ["money ceremony", () => money.killRun("r", "x"), { kind: "no_environment" }],
    ["policy", () => policy.fetchApprovals(), { kind: "no_environment" }],
    ["policy ceremony", () => policy.alwaysAllowTool("a"), { kind: "no_environment" }],
    ["remote", () => remote.connectTunnel(), { kind: "internal", message: "no backend available" }],
    ["identity", () => identity.fetchAlerts(), { kind: "no_environment" }],
    ["memory", () => memory.fetchWhy("m"), { kind: "no_environment" }],
    ["quality", () => quality.fetchBaselines(), { kind: "no_environment" }],
    ["crypto", () => crypto.scanCbom("/"), { kind: "no_environment" }],
    ["drills", () => drills.runDrills("/", "", false, ""), { kind: "no_environment" }],
    ["evidence", () => evidence.buildEvidence({} as never), { kind: "build", message: "no backend available" }],
    ["admission", () => admission.runAdmissionCheck("k", "a"), { kind: "no_environment" }],
    ["onboard", () => onboard.fetchOnboardStatus(), { kind: "no_environment", message: "no backend available" }],
    ["delegation", () => delegation.revokeDelegation("s", "r"), { kind: "no_environment" }],
    ["egress", () => egress.fetchEgress(), { kind: "no_environment" }],
  ];
  it.each(cases)("%s refuses with its no-environment error", async (_l, run, expected) => {
    box.present = false;
    await expect(run()).rejects.toEqual(expected);
    expect(box.calls).toEqual([]);
  });

  it("the reads that may say nothing say nothing", async () => {
    box.present = false;
    expect(await entityRecords.fetchUserRecord("u")).toBeNull();
    expect(await entityRecords.fetchUnitRecord("t")).toBeNull();
    expect(await entityRecords.blockUser("u", true)).toBeNull();
    expect(await entityRecords.blockUnit("t", true)).toBeNull();
    expect(await agentProfile.fetchAgentProfile("a")).toBeNull();
    expect(await agentStops.fetchAgentStops("a")).toBeNull();
    expect(await quarantine.fetchQuarantine()).toBeNull();
    expect(await replay.fetchRunEvents("r", 10)).toEqual([]);
    expect(await busStatus.fetchBusMode()).toBeNull();
    expect(await delegation.stopAgent("a", "r")).toEqual({ ok: false, error: { kind: "failed", detail: "no console backend in this build" } });
    expect(await delegation.liftAgent("a", "r")).toEqual({ ok: false, error: { kind: "failed", detail: "no console backend in this build" } });
    expect(box.calls).toEqual([]);
  });

  it("each status read answers the plane's own unavailable status", async () => {
    box.present = false;
    expect(await money.fetchMoneyStatus()).toEqual({ state: "no_environment" });
    expect(await policy.fetchPolicyStatus()).toEqual({ state: "no_environment" });
    expect(await identity.fetchIdentityStatus()).toEqual({ state: "no_environment" });
    expect((await admission.fetchAdmissionStatus()).gateway).toEqual({ state: "no_environment" });
    expect((await remote.fetchRemoteStatus()).environment).toBeNull();
    expect((await evidence.fetchEvidenceStatus()).qryx_available).toBe(false);
  });
});

describe("a refusal is normalised once, the same way in every plane", () => {
  const planes: [string, () => Promise<unknown>, (msg: string) => unknown][] = [
    ["money", () => money.fetchRuns(), (m) => ({ kind: "cloud", status: null, message: m })],
    ["money ceremony", () => money.setBudget("r", 1, "x"), (m) => ({ kind: "cloud", status: null, message: m })],
    ["policy", () => policy.fetchPolicies(), (m) => ({ kind: "wardryx", status: null, message: m })],
    ["policy ceremony", () => policy.decideApproval("a", "approve" as never), (m) => ({ kind: "wardryx", status: null, message: m })],
    ["remote", () => remote.stopRemoteTail(), (m) => ({ kind: "internal", message: m })],
    ["identity", () => identity.rescan(), (m) => ({ kind: "idryx", status: null, message: m })],
    ["memory", () => memory.forget("m"), (m) => ({ kind: "mcp", message: m })],
    ["quality", () => quality.fetchRunScores("r"), (m) => ({ kind: "verdryx", message: m })],
    ["crypto", () => crypto.scanNcsc("/"), (m) => ({ kind: "qryx", message: m })],
    ["drills", () => drills.runDrills("/", "", false, ""), (m) => ({ kind: "mockryx", message: m })],
    ["evidence", () => evidence.buildEvidence({} as never), (m) => ({ kind: "build", message: m })],
    ["admission", () => admission.runAdmissionCheck("k", "a"), (m) => ({ kind: "gateway", status: null, message: m })],
    ["onboard", () => onboard.generateOnboardBundle({} as never), (m) => ({ kind: "transport", message: m })],
    ["delegation", () => delegation.revokeDelegation("s", "r"), (m) => ({ kind: "unreachable", detail: m })],
    ["egress", () => egress.fetchEgress(), (m) => ({ kind: "backend", message: m })],
  ];

  it.each(planes)("%s: an Error or a bare string becomes the catch-all kind with its message", async (_l, run, catchAll) => {
    box.answer = reject(new Error("connection reset"));
    await expect(run()).rejects.toEqual(catchAll("connection reset"));
    box.answer = reject("upstream said no");
    await expect(run()).rejects.toEqual(catchAll("upstream said no"));
  });

  it.each(planes)("%s: the plane's own structured error passes through unchanged", async (_l, run) => {
    const own = { kind: "validation", message: "bad field" };
    box.answer = reject(own);
    await expect(run()).rejects.toEqual(own);
  });

  const gated: [string, () => Promise<unknown>][] = [
    ["money", () => money.fetchRuns()],
    ["money ceremony", () => money.killRun("r", "x")],
    ["policy", () => policy.fetchApprovals()],
    ["policy ceremony", () => policy.alwaysAllowTool("a")],
    ["remote", () => remote.connectTunnel()],
    ["delegation", () => delegation.revokeDelegation("s", "r")],
  ];
  it.each(gated)("%s: the role gate's 403 becomes role_required naming the role", async (_l, run) => {
    box.answer = reject({ error: "role admin required" });
    await expect(run()).rejects.toEqual({ kind: "role_required", role: "admin" });
  });

  it("stopping and lifting an agent answer a result, never a throw", async () => {
    box.answer = () => ({ stopped: true });
    expect(await delegation.stopAgent("agent://a/b", "leaver")).toEqual({ ok: true, outcome: { stopped: true } });
    expect(box.calls[0]).toEqual({ via: "ceremony", command: "agent_stop", args: { subject: "agent://a/b", reason: "leaver" } });
    expect(await delegation.liftAgent("agent://a/b", "back")).toEqual({ ok: true, outcome: { stopped: true } });
    expect(box.calls[1]).toEqual({ via: "ceremony", command: "agent_lift", args: { subject: "agent://a/b", reason: "back" } });

    box.answer = reject({ error: "role approver required" });
    expect(await delegation.stopAgent("a", "r")).toEqual({ ok: false, error: { kind: "role_required", role: "approver" } });
    box.answer = reject({ kind: "ban_refused", detail: "x" });
    expect(await delegation.liftAgent("a", "r")).toEqual({ ok: false, error: { kind: "ban_refused", detail: "x" } });
    box.answer = reject(new Error("socket hang up"));
    expect(await delegation.stopAgent("a", "r")).toEqual({ ok: false, error: { kind: "failed", detail: "socket hang up" } });
  });
});

describe("a status read the box fails is a settled, named state", () => {
  beforeEach(() => {
    box.answer = reject(new Error("refused"));
  });
  it("money, policy, identity, memory, quality and admission say unreachable with the reason", async () => {
    expect(await money.fetchMoneyStatus()).toMatchObject({ state: "pairing_failed", reason: "refused" });
    expect(await policy.fetchPolicyStatus()).toMatchObject({ reason: "refused" });
    expect(await identity.fetchIdentityStatus()).toMatchObject({ state: "unreachable", reason: "refused" });
    expect(await memory.fetchMemoryStatus()).toMatchObject({ reason: "refused" });
    expect(await quality.fetchQualityStatus()).toMatchObject({ reason: "refused" });
    expect((await admission.fetchAdmissionStatus()).gateway).toMatchObject({ state: "unreachable" });
  });
  it("crypto, drills, evidence and remote fall back to their unavailable status", async () => {
    expect(await crypto.fetchCryptoStatus()).toEqual({ state: "no_environment" });
    expect(await drills.fetchDrillsStatus()).toEqual({ state: "no_environment" });
    expect((await evidence.fetchEvidenceStatus()).tokenfuse_available).toBe(false);
    expect((await remote.fetchRemoteStatus()).tunnel).toEqual({ state: "disconnected" });
  });
});

describe("the reads that may say nothing", () => {
  it("hand back what the box answered, and null or empty when it failed", async () => {
    box.answer = (c) => (c === "user_record" ? { handle: "u" } : c === "unit_record" ? { team: "t" } : null);
    expect(await entityRecords.fetchUserRecord("u")).toEqual({ handle: "u" });
    expect(await entityRecords.fetchUnitRecord("t")).toEqual({ team: "t" });
    expect(box.calls.map((c) => [c.command, c.args])).toEqual([
      ["user_record", { user: "u" }],
      ["unit_record", { team: "t" }],
    ]);

    box.answer = reject(new Error("x"));
    expect(await entityRecords.fetchUserRecord("u")).toBeNull();
    expect(await entityRecords.fetchUnitRecord("t")).toBeNull();
    expect(await entityRecords.blockUser("u", true)).toBeNull();
    expect(await entityRecords.blockUnit("t", false)).toBeNull();
    expect(await agentProfile.fetchAgentProfile("a")).toBeNull();
    expect(await agentStops.fetchAgentStops("a")).toBeNull();
    expect(await quarantine.fetchQuarantine()).toBeNull();
    expect(await replay.fetchRunEvents("r", 10)).toEqual([]);
    expect(await busStatus.fetchBusMode()).toBeNull();
  });

  it("refuse an answer that is not the panel they asked for, rather than render it", async () => {
    box.answer = () => ({ something: "else" });
    expect(await agentProfile.fetchAgentProfile("a")).toBeNull();
    expect(await agentStops.fetchAgentStops("a")).toBeNull();
    expect(await quarantine.fetchQuarantine()).toBeNull();
    await expect(egress.fetchEgress()).rejects.toMatchObject({ kind: "backend", message: expect.stringContaining("not a report that your agents made no web requests") });
    box.answer = () => null;
    expect(await agentProfile.fetchAgentProfile("a")).toBeNull();
  });

  it("pass a well-formed panel through and send the asked-for arguments", async () => {
    box.answer = (c) =>
      c === "agent_profile" ? { confidence: "normal" } : c === "run_events" ? [{ id: 1 }] : c === "bus_status" ? { kind: "demo", dir: "/d" } : { measured: true };
    expect(await agentProfile.fetchAgentProfile("agent://a/b")).toEqual({ confidence: "normal" });
    expect(await agentProfile.fetchAgentProfile("agent://a/b", 7)).toEqual({ confidence: "normal" });
    expect(await agentStops.fetchAgentStops("agent://a/b")).toEqual({ measured: true });
    expect(await quarantine.fetchQuarantine()).toEqual({ measured: true });
    expect(await replay.fetchRunEvents("run-1", 25)).toEqual([{ id: 1 }]);
    expect(await busStatus.fetchBusMode()).toEqual({ kind: "demo", dir: "/d" });
    expect(box.calls.map((c) => [c.command, c.args])).toEqual([
      ["agent_profile", { agent_id: "agent://a/b", window_days: agentProfile.PROFILE_WINDOW_DAYS }],
      ["agent_profile", { agent_id: "agent://a/b", window_days: 7 }],
      ["agent_stops", { agent_id: "agent://a/b" }],
      ["bus_quarantine", undefined],
      ["run_events", { run_id: "run-1", limit: 25 }],
      ["bus_status", undefined],
    ]);
  });

  it("a block that lands tells every open panel to re-read", async () => {
    const heard = vi.fn();
    window.addEventListener(CONSOLE_STATE_CHANGED_EVENT, heard);
    box.answer = () => ({ handle: "u", stopped: true });
    await entityRecords.blockUser("u", true);
    await entityRecords.blockUnit("t", true);
    await money.killRun("run-1", "x");
    expect(heard).toHaveBeenCalledTimes(3);
    expect(box.calls.map((c) => [c.command, c.args])).toEqual([
      ["user_block", { user: "u", blocked: true }],
      ["unit_block", { team: "t", blocked: true }],
      ["money_kill_run", { run_id: "run-1", reason: "x" }],
    ]);
    box.answer = reject(new Error("x"));
    await entityRecords.blockUser("u", true);
    expect(heard).toHaveBeenCalledTimes(3);
    window.removeEventListener(CONSOLE_STATE_CHANGED_EVENT, heard);
  });
});

describe("the error sentences an operator reads", () => {
  it("money", () => {
    expect(money.describeMoneyError({ kind: "bootstrapping" } as never)).toBe("Still connecting to the Cloud environment.");
    expect(money.describeMoneyError({ kind: "no_environment" })).toBe("No TokenFuse Cloud environment found.");
    expect(money.describeMoneyError({ kind: "pairing_failed", reason: "bad key" } as never)).toBe("Pairing failed: bad key");
    expect(money.describeMoneyError({ kind: "plan_required", feature: "kill", org: "acme" } as never)).toBe("The Cloud plane refused kill for acme.");
    expect(money.describeMoneyError({ kind: "break_glass_missing_reason" } as never)).toMatch(/non-empty reason/);
    expect(money.describeMoneyError({ kind: "invalid_argument", message: "negative" } as never)).toBe("Not sent: negative");
    expect(money.describeMoneyError({ kind: "cloud", status: 502, message: "bad gateway" })).toBe("Cloud error 502: bad gateway");
    expect(money.describeMoneyError({ kind: "cloud", status: null, message: "bad gateway" })).toBe("bad gateway");
    expect(money.describeMoneyError({ kind: "role_required", role: "admin" })).toBe("This action needs the admin role.");
    expect(money.describeMoneyError({ kind: "role_required", role: "admin" }, "viewer")).toBe(
      "This action needs the admin role. You are signed in as viewer.",
    );
  });

  it("policy", () => {
    expect(policy.describePolicyError({ kind: "bootstrapping" } as never)).toMatch(/Still connecting/);
    expect(policy.describePolicyError({ kind: "no_environment" })).toBe("No Wardryx policy plane found.");
    expect(policy.describePolicyError({ kind: "unreachable", reason: "dns" } as never)).toBe("Could not reach Wardryx: dns");
    expect(policy.describePolicyError({ kind: "approver_statement", reason: "no key" } as never)).toBe("Refused before it reached Wardryx: no key");
    expect(policy.describePolicyError({ kind: "standing_grant", reason: "expired" } as never)).toBe("Refused before it reached Wardryx: expired");
    expect(policy.describePolicyError({ kind: "wardryx", status: 409, message: "decided" })).toBe("Wardryx error 409: decided");
    expect(policy.describePolicyError({ kind: "wardryx", status: null, message: "decided" })).toBe("decided");
    expect(policy.describePolicyError({ kind: "role_required", role: "approver" }, "viewer")).toBe(
      "This action needs the approver role. You are signed in as viewer.",
    );
    expect(policy.describePolicyError({ kind: "role_required", role: "approver" })).toBe("This action needs the approver role.");
  });

  it("remote", () => {
    expect(remote.describeRemoteError({ kind: "bootstrapping" } as never)).toMatch(/Still resolving/);
    expect(remote.describeRemoteError({ kind: "no_environment" } as never)).toMatch(/No remote environment saved/);
    expect(remote.describeRemoteError({ kind: "no_wireguard_go_binary" } as never)).toMatch(/wireguard-go/);
    for (const kind of ["invalid", "wg_server_not_configured", "wg", "ssh", "hetzner", "cloud", "internal"]) {
      expect(remote.describeRemoteError({ kind, message: `m-${kind}` } as never)).toBe(`m-${kind}`);
    }
    expect(remote.describeRemoteError({ kind: "role_required", role: "admin" })).toBe("This action needs the admin role.");
    expect(remote.describeRemoteError({ kind: "role_required", role: "admin" }, "approver")).toMatch(/signed in as approver/);
  });

  it("identity, memory, quality, crypto, drills, evidence", () => {
    expect(identity.describeIdentityError({ kind: "bootstrapping" } as never)).toMatch(/Still connecting/);
    expect(identity.describeIdentityError({ kind: "no_environment" })).toBe("No Idryx identity plane found.");
    expect(identity.describeIdentityError({ kind: "unreachable", reason: "x" } as never)).toBe("Could not reach idryx: x");
    expect(identity.describeIdentityError({ kind: "idryx", status: 500, message: "y" })).toBe("Idryx error 500: y");
    expect(identity.describeIdentityError({ kind: "idryx", status: null, message: "y" })).toBe("y");
    expect(identity.describeIdentityError({ kind: "rescan_unavailable" } as never)).toMatch(/~\/\.taipan\/bin\/idryx/);

    expect(memory.describeMemoryError({ kind: "bootstrapping" } as never)).toMatch(/Engram/);
    expect(memory.describeMemoryError({ kind: "no_environment" })).toBe("No Engram memory plane found.");
    expect(memory.describeMemoryError({ kind: "unreachable", reason: "spawn" } as never)).toBe("Could not start engram-mcp: spawn");
    expect(memory.describeMemoryError({ kind: "mcp", message: "z" })).toBe("z");

    expect(quality.describeQualityError({ kind: "bootstrapping" } as never)).toBeTruthy();
    expect(quality.describeQualityError({ kind: "no_environment" })).toBeTruthy();
    expect(quality.describeQualityError({ kind: "unreachable", reason: "q" } as never)).toContain("q");
    expect(quality.describeQualityError({ kind: "verdryx", message: "v" })).toBe("v");

    expect(crypto.describeCryptoError({ kind: "bootstrapping" } as never)).toBeTruthy();
    expect(crypto.describeCryptoError({ kind: "no_environment" })).toBeTruthy();
    expect(crypto.describeCryptoError({ kind: "qryx", message: "c" })).toBe("c");

    expect(drills.describeDrillsError({ kind: "bootstrapping" } as never)).toMatch(/mockryx/);
    expect(drills.describeDrillsError({ kind: "no_environment" })).toBe("No drills plane found.");
    expect(drills.describeDrillsError({ kind: "mockryx", message: "d" })).toBe("d");

    expect(evidence.describeEvidenceError({ kind: "bootstrapping" } as never)).toMatch(/evidence sources/);
    expect(evidence.describeEvidenceError({ kind: "build", message: "e" })).toBe("e");
  });

  it("admission", () => {
    const d = admission.describeAdmissionError;
    expect(d({ kind: "bootstrapping" } as never)).toBe("Still connecting to the gateway.");
    expect(d({ kind: "no_environment" })).toBe("No gateway found in this environment.");
    expect(d({ kind: "unreachable", reason: "r" } as never)).toBe("Could not reach the gateway: r");
    expect(d({ kind: "gateway", status: 401, message: "m" })).toBe("Gateway error 401: m");
    expect(d({ kind: "gateway", status: null, message: "m" })).toBe("m");
    expect(d({ kind: "verdryx_bin_missing", path: "/opt/v" } as never)).toMatch(/^verdryx binary not found at \/opt\/v/);
    expect(d({ kind: "verdryx_db_missing" } as never)).toMatch(/No verdryx.db/);
    expect(d({ kind: "verdryx", message: "v" } as never)).toBe("v");
    expect(d({ kind: "unparseable_output", context: "score", stdout_excerpt: "garbage" } as never)).toBe(
      "Could not read verdryx's score from its output: garbage",
    );
    expect(d({ kind: "run_not_found", run_id: "e1" } as never)).toMatch(/eval run e1/);
  });

  it("onboard, and which io error is an existing file", () => {
    expect(onboard.describeOnboardError({ kind: "no_environment", message: "" })).toMatch(/No backend available/);
    expect(onboard.describeOnboardError({ kind: "transport", message: "t" })).toBe("Could not reach the console backend: t");
    expect(onboard.describeOnboardError({ kind: "validation", message: "bad id" } as never)).toBe("bad id");
    expect(onboard.describeOnboardError({ kind: "io", message: "File exists: /p" } as never)).toBe("File exists: /p");
    expect(onboard.describeOnboardError({ kind: "io", message: "permission denied" } as never)).toBe(
      "Local filesystem error: permission denied",
    );
    expect(onboard.describeOnboardError({ kind: "other", message: "o" } as never)).toBe("o");
    expect(onboard.isExistingFileError({ kind: "io", message: "already EXISTS" } as never)).toBe(true);
    expect(onboard.isExistingFileError({ kind: "validation", message: "exists" } as never)).toBe(false);
  });

  it("an onboard error without a message is not taken for a structured one", async () => {
    box.answer = reject({ kind: "io" });
    await expect(onboard.generateOnboardBundle({} as never)).rejects.toEqual({ kind: "transport", message: "[object Object]" });
  });
});

describe("the small judgements the panels print", () => {
  it("an agent's profile sentence says what the window can and cannot show", () => {
    const base: Omit<agentProfile.AgentProfile, "confidence"> = {
      agent_id: "a",
      days_held: 40,
      total: 120,
      median_day: 2.5,
      latest_full_day: 9,
      times_median: 3.6,
      busiest_day_share: 0,
      direction: "rising",
      top_type: null,
      top_type_share: 0,
      daily: [],
    };
    expect(agentProfile.profileSentence({ ...base, confidence: "no_data" })).toMatch(/^No events stored/);
    expect(agentProfile.profileSentence({ ...base, confidence: "too_new", days_held: 3, total: 7 })).toBe(
      "Watched for 3 day(s), which is too short to call anything unusual. 7 event(s) so far.",
    );
    expect(agentProfile.profileSentence({ ...base, confidence: "normal" })).toBe(
      "Its median day over 40 days is 2.5 event(s); yesterday was 9, which is 3.6x its median day.",
    );
    expect(agentProfile.profileSentence({ ...base, confidence: "normal", median_day: 2, times_median: 4.4444 })).toBe(
      "Its median day over 40 days is 2 event(s); yesterday was 9, which is 4.44x its median day.",
    );
    expect(agentProfile.profileSentence({ ...base, confidence: "normal", times_median: null })).toMatch(
      /Most days are empty, so a multiple would say nothing/,
    );
  });

  it("a stop names who stopped it and why, or says nobody recorded it", () => {
    const e = { ts: "t", type_: "kill", source: "tokenfuse", by_operator: true };
    expect(agentStops.stopSummary({ ...e, actor: "ops@x", reason: "runaway" })).toBe("by ops@x: runaway");
    expect(agentStops.stopSummary({ ...e, actor: null, reason: null })).toBe("by the services, no reason recorded");
  });

  it("the navigation-only share is a ratio of fetched requests, and nothing when none were fetched", () => {
    const panel = (fetched: number, navigation_only: number) => ({ totals: { fetched, navigation_only } }) as never;
    expect(egress.navigationOnlyShare(panel(0, 0))).toBeNull();
    expect(egress.navigationOnlyShare(panel(8, 2))).toBe(0.25);
  });
});
