import { describe, expect, it } from "vitest";
import type { IdryxAlert, IdryxIdentity, IdryxPermission } from "../identityTypes";
import type { PolicyRecord } from "../policyTypes";
import {
  buildAccessRows,
  compileGlob,
  effectiveOverlay,
  isAgentIdentity,
  matchedPolicies,
  matchesGlob,
  mcpReachForAgent,
  mcpServerIdentities,
  permissionRollup,
  shadowServerIds,
  sortAccessRowsWorstFirst,
} from "./access";

// ---------------------------------------------------------------------------
// Test fixtures - minimal, valid instances of each wire type, overridable.
// ---------------------------------------------------------------------------

function policy(overrides: Partial<PolicyRecord> & { id: string; target: string }): PolicyRecord {
  return {
    id: overrides.id,
    name: overrides.name ?? "",
    target: overrides.target,
    deny_tool: overrides.deny_tool ?? [],
    allow_domains: overrides.allow_domains ?? [],
    require_human_above_usd: overrides.require_human_above_usd ?? 0,
    deny_above_usd: overrides.deny_above_usd ?? 0,
    max_steps: overrides.max_steps ?? 0,
    deny_if_unattested: overrides.deny_if_unattested ?? false,
    updated_at: overrides.updated_at ?? null,
  };
}

function permission(name: string, admin = false, used = false): IdryxPermission {
  return { name, admin, used };
}

function identity(overrides: Partial<IdryxIdentity> & { id: string }): IdryxIdentity {
  return {
    id: overrides.id,
    type: overrides.type ?? "agent",
    privileged: overrides.privileged ?? false,
    source: overrides.source ?? "tokenfuse",
    owner: overrides.owner ?? "user://acme/alice",
    created: overrides.created ?? "",
    last_used: overrides.last_used ?? "",
    runtime: overrides.runtime ?? "",
    on_behalf_of: overrides.on_behalf_of ?? [],
    permissions: overrides.permissions ?? [],
    remediation: overrides.remediation ?? null,
    rotation: overrides.rotation ?? null,
    events: overrides.events ?? 0,
    alerts: overrides.alerts ?? 0,
  };
}

function alert(overrides: Partial<IdryxAlert> & { detector: string; identity: string }): IdryxAlert {
  return {
    detector: overrides.detector,
    identity: overrides.identity,
    severity: overrides.severity ?? "high",
    time: overrides.time ?? "2026-07-01T00:00:00Z",
    summary: overrides.summary ?? "",
  };
}

// ---------------------------------------------------------------------------
// Glob semantics (policy.go's compileGlob, ported verbatim)
// ---------------------------------------------------------------------------

describe("compileGlob / matchesGlob", () => {
  it("'*' matches any run of characters, including across '/'", () => {
    expect(matchesGlob("agent://acme/team/sub/name", "agent://acme/*")).toBe(true);
    expect(matchesGlob("agent://acme/name", "agent://acme/*")).toBe(true);
    expect(matchesGlob("agent://acme/", "agent://acme/*")).toBe(true);
  });

  it("'?' matches exactly one character, no more and no fewer", () => {
    expect(matchesGlob("agent://acme/abc", "agent://acme/a?c")).toBe(true);
    expect(matchesGlob("agent://acme/ac", "agent://acme/a?c")).toBe(false);
    expect(matchesGlob("agent://acme/abbc", "agent://acme/a?c")).toBe(false);
  });

  it("anchors both ends - a longer or shorter string never matches", () => {
    expect(matchesGlob("agent://acme/foo", "agent://acme/foo")).toBe(true);
    expect(matchesGlob("agent://acme/foobar", "agent://acme/foo")).toBe(false);
    expect(matchesGlob("xagent://acme/foo", "agent://acme/foo")).toBe(false);
  });

  it("escapes regex metacharacters in the pattern - '.' is literal, not 'any char'", () => {
    expect(matchesGlob("agent://acme.example/x", "agent://acme.example/*")).toBe(true);
    expect(matchesGlob("agent://acmeXexample/x", "agent://acme.example/*")).toBe(false);
  });

  it("treats an empty pattern as matching nothing rather than throwing", () => {
    expect(compileGlob("")).toBeNull();
    expect(matchesGlob("agent://acme/foo", "")).toBe(false);
  });
});

// ---------------------------------------------------------------------------
// matchedPolicies: filter + deterministic (target, name) order
// ---------------------------------------------------------------------------

describe("matchedPolicies", () => {
  it("keeps only policies whose target glob matches the agent id", () => {
    const policies = [
      policy({ id: "a", target: "agent://acme/sre/*" }),
      policy({ id: "b", target: "agent://acme/finops/*" }),
    ];
    const matched = matchedPolicies("agent://acme/sre/rca-copilot", policies);
    expect(matched.map((p) => p.id)).toEqual(["a"]);
  });

  it("sorts by target then name, regardless of input order (mirrors wardryx normalize)", () => {
    const policies = [
      policy({ id: "z", name: "zeta", target: "agent://acme/*" }),
      policy({ id: "a", name: "alpha", target: "agent://acme/*" }),
      policy({ id: "m", name: "alpha", target: "agent://acme/aaa" }),
    ];
    // Byte-wise target comparison (Go's plain `<`, ported as-is): '*' (0x2A)
    // sorts before 'a' (0x61), so the glob "agent://acme/*" sorts before the
    // exact "agent://acme/aaa" - target order is a literal string compare,
    // never a "more specific glob wins" heuristic. Within the shared target,
    // "alpha" sorts before "zeta".
    const matchedOrderA = matchedPolicies("agent://acme/aaa", [...policies]);
    const matchedOrderB = matchedPolicies("agent://acme/aaa", [...policies].reverse());
    expect(matchedOrderA.map((p) => p.id)).toEqual(["a", "z", "m"]);
    expect(matchedOrderB.map((p) => p.id)).toEqual(["a", "z", "m"]);
  });

  it("sorts a blank name as the policy's own target (normalize's default), for ordering only", () => {
    const policies = [
      policy({ id: "named", name: "aaa-explicit", target: "agent://acme/*" }),
      policy({ id: "blank", name: "", target: "agent://acme/*" }),
    ];
    const matched = matchedPolicies("agent://acme/x", policies);
    // blank name's effective sort key is its own target ("agent://acme/*"),
    // which sorts after "aaa-explicit" - but the displayed `name` field
    // itself must still come back exactly as stored ("").
    expect(matched.map((p) => p.id)).toEqual(["named", "blank"]);
    expect(matched.find((p) => p.id === "blank")?.name).toBe("");
  });
});

// ---------------------------------------------------------------------------
// effectiveOverlay: union/intersection/min/any composition (pdp.go)
// ---------------------------------------------------------------------------

describe("effectiveOverlay", () => {
  it("unions deny_tool across every matched policy, deduped and sorted", () => {
    const matched = [
      policy({ id: "a", target: "*", deny_tool: ["shell_exec", "prod_delete"] }),
      policy({ id: "b", target: "*", deny_tool: ["prod_delete", "external_send"] }),
    ];
    expect(effectiveOverlay(matched).deniedTools).toEqual(["external_send", "prod_delete", "shell_exec"]);
  });

  it("reports 'unrestricted' when no matched policy sets allow_domains", () => {
    const matched = [policy({ id: "a", target: "*", deny_tool: ["x"] })];
    expect(effectiveOverlay(matched).allowDomains.effective).toEqual({ kind: "unrestricted" });
  });

  it("intersects (not unions) allow_domains across matched policies with a non-empty list", () => {
    const matched = [
      policy({ id: "a", target: "*", allow_domains: ["api.github.com", "registry.npmjs.org"] }),
      policy({ id: "b", target: "*", allow_domains: ["registry.npmjs.org"] }),
    ];
    const overlay = effectiveOverlay(matched);
    expect(overlay.allowDomains.effective).toEqual({ kind: "restricted", domains: ["registry.npmjs.org"] });
    expect(overlay.allowDomains.perPolicy).toEqual([
      { policyId: "a", policyName: "", policyTarget: "*", domains: ["api.github.com", "registry.npmjs.org"] },
      { policyId: "b", policyName: "", policyTarget: "*", domains: ["registry.npmjs.org"] },
    ]);
  });

  it("distinguishes a genuine contradiction (restricted, but nothing in common) from unrestricted", () => {
    const matched = [
      policy({ id: "a", target: "*", allow_domains: ["only-a.example"] }),
      policy({ id: "b", target: "*", allow_domains: ["only-b.example"] }),
    ];
    const eff = effectiveOverlay(matched).allowDomains.effective;
    expect(eff).toEqual({ kind: "restricted", domains: [] });
    expect(eff).not.toEqual({ kind: "unrestricted" });
  });

  it("a policy with an empty allow_domains list imposes no restriction and is excluded from perPolicy", () => {
    const matched = [
      policy({ id: "a", target: "*", allow_domains: [] }),
      policy({ id: "b", target: "*", allow_domains: ["only-b.example"] }),
    ];
    const overlay = effectiveOverlay(matched);
    expect(overlay.allowDomains.perPolicy.map((p) => p.policyId)).toEqual(["b"]);
    expect(overlay.allowDomains.effective).toEqual({ kind: "restricted", domains: ["only-b.example"] });
  });

  it("takes the smallest POSITIVE max_steps/require_human_above_usd/deny_above_usd, ignoring zero (unset)", () => {
    const matched = [
      policy({ id: "a", target: "*", max_steps: 0, require_human_above_usd: 0, deny_above_usd: 100 }),
      policy({ id: "b", target: "*", max_steps: 12, require_human_above_usd: 25, deny_above_usd: 40 }),
      policy({ id: "c", target: "*", max_steps: 30, require_human_above_usd: 10, deny_above_usd: 0 }),
    ];
    const overlay = effectiveOverlay(matched);
    expect(overlay.maxSteps).toBe(12);
    expect(overlay.requireHumanAboveUsd).toBe(10);
    expect(overlay.denyAboveUsd).toBe(40);
  });

  it("yields null for every ceiling when no matched policy sets a positive value", () => {
    const matched = [policy({ id: "a", target: "*" })];
    const overlay = effectiveOverlay(matched);
    expect(overlay.maxSteps).toBeNull();
    expect(overlay.requireHumanAboveUsd).toBeNull();
    expect(overlay.denyAboveUsd).toBeNull();
  });

  it("deny_if_unattested is true if ANY matched policy sets it", () => {
    const matched = [
      policy({ id: "a", target: "*", deny_if_unattested: false }),
      policy({ id: "b", target: "*", deny_if_unattested: true }),
    ];
    expect(effectiveOverlay(matched).denyIfUnattested).toBe(true);
    expect(effectiveOverlay([matched[0]]).denyIfUnattested).toBe(false);
  });

  it("returns every vacuous default for an empty matched list", () => {
    const overlay = effectiveOverlay([]);
    expect(overlay.deniedTools).toEqual([]);
    expect(overlay.allowDomains.effective).toEqual({ kind: "unrestricted" });
    expect(overlay.maxSteps).toBeNull();
    expect(overlay.requireHumanAboveUsd).toBeNull();
    expect(overlay.denyAboveUsd).toBeNull();
    expect(overlay.denyIfUnattested).toBe(false);
  });
});

// ---------------------------------------------------------------------------
// permissionRollup: the honesty gate (least_privilege.go)
// ---------------------------------------------------------------------------

describe("permissionRollup", () => {
  it("counts granted/used/unused and flags hasUsageSignal when some permission was used", () => {
    const rollup = permissionRollup([
      permission("pagerduty_read", false, true),
      permission("pagerduty_admin", true, false),
      permission("incident_write", false, false),
    ]);
    expect(rollup.granted).toBe(3);
    expect(rollup.used).toBe(1);
    expect(rollup.unused.map((p) => p.name)).toEqual(["pagerduty_admin", "incident_write"]);
    expect(rollup.adminUnused.map((p) => p.name)).toEqual(["pagerduty_admin"]);
    expect(rollup.hasUsageSignal).toBe(true);
  });

  it("hasUsageSignal is false when NO permission is used - even though every one is nominally 'unused'", () => {
    const rollup = permissionRollup([permission("metrics_read", false, false), permission("alerts_dedupe", false, false)]);
    expect(rollup.hasUsageSignal).toBe(false);
    // The raw counts are still honest facts, just not a "highlight-worthy"
    // signal without usage data - the UI layer is what must not escalate
    // this, not this pure rollup.
    expect(rollup.unused).toHaveLength(2);
  });

  it("is vacuously clean for zero permissions", () => {
    const rollup = permissionRollup([]);
    expect(rollup).toMatchObject({ granted: 0, used: 0, hasUsageSignal: false });
    expect(rollup.unused).toEqual([]);
    expect(rollup.adminUnused).toEqual([]);
  });
});

// ---------------------------------------------------------------------------
// Shadow derivation from alerts (idryx exposes no shadow flag over REST)
// ---------------------------------------------------------------------------

describe("shadowServerIds", () => {
  it("derives the shadow set from shadow_mcp alerts only, ignoring every other detector", () => {
    const alerts = [
      alert({ detector: "shadow_mcp", identity: "mcp://acme/shadow/one" }),
      alert({ detector: "agent_shadow_tool", identity: "agent://acme/sre/x" }),
      alert({ detector: "runaway_agent", identity: "mcp://acme/shadow/one" }),
    ];
    const ids = shadowServerIds(alerts);
    expect(ids.has("mcp://acme/shadow/one")).toBe(true);
    expect(ids.has("agent://acme/sre/x")).toBe(false);
    expect(ids.size).toBe(1);
  });

  it("is empty when there are no shadow_mcp alerts", () => {
    expect(shadowServerIds([]).size).toBe(0);
  });
});

describe("mcpServerIdentities", () => {
  it("keeps only type === 'mcp_server' identities", () => {
    const identities = [
      identity({ id: "agent://acme/sre/x", type: "agent" }),
      identity({ id: "mcp://acme/one", type: "mcp_server" }),
      identity({ id: "user://acme/alice", type: "human" }),
    ];
    expect(mcpServerIdentities(identities).map((i) => i.id)).toEqual(["mcp://acme/one"]);
  });
});

// ---------------------------------------------------------------------------
// MCP reach: the permission-name-intersection join (agent_shadow_tool.go)
// ---------------------------------------------------------------------------

describe("mcpReachForAgent", () => {
  const sanctioned = identity({
    id: "mcp://acme/sanctioned/pagerduty",
    type: "mcp_server",
    permissions: [permission("pagerduty_read"), permission("pagerduty_admin", true)],
  });
  const shadow = identity({
    id: "mcp://acme/shadow/scratch",
    type: "mcp_server",
    permissions: [permission("scratch_notes_write")],
  });
  const shadowIds = new Set([shadow.id]);

  it("splits reach into sanctioned vs shadow by name-intersection with each server's permissions", () => {
    const reach = mcpReachForAgent(["pagerduty_read", "scratch_notes_write", "unrelated_perm"], [sanctioned, shadow], shadowIds);
    expect(reach.sanctionedTools).toEqual(["pagerduty_read"]);
    expect(reach.shadowTools).toEqual(["scratch_notes_write"]);
    expect(reach.sanctionedServers).toEqual([{ serverId: sanctioned.id, tools: ["pagerduty_read"] }]);
    expect(reach.shadowServers).toEqual([{ serverId: shadow.id, tools: ["scratch_notes_write"] }]);
  });

  it("excludes a server entirely when the agent shares no permission name with it", () => {
    const reach = mcpReachForAgent(["unrelated_perm"], [sanctioned, shadow], shadowIds);
    expect(reach.sanctionedServers).toEqual([]);
    expect(reach.shadowServers).toEqual([]);
    expect(reach.sanctionedTools).toEqual([]);
    expect(reach.shadowTools).toEqual([]);
  });

  it("is empty when the agent has no permissions at all", () => {
    const reach = mcpReachForAgent([], [sanctioned, shadow], shadowIds);
    expect(reach.sanctionedTools).toEqual([]);
    expect(reach.shadowTools).toEqual([]);
  });
});

// ---------------------------------------------------------------------------
// isAgentIdentity: the console's one recognized "agent" type
// ---------------------------------------------------------------------------

describe("isAgentIdentity", () => {
  it("is true only for type === 'agent'", () => {
    expect(isAgentIdentity(identity({ id: "a", type: "agent" }))).toBe(true);
    expect(isAgentIdentity(identity({ id: "b", type: "mcp_server" }))).toBe(false);
    expect(isAgentIdentity(identity({ id: "c", type: "service_account" }))).toBe(false);
    expect(isAgentIdentity(identity({ id: "d", type: "human" }))).toBe(false);
  });
});

// ---------------------------------------------------------------------------
// buildAccessRows: the full row assembly, including the null-policies honesty gate
// ---------------------------------------------------------------------------

describe("buildAccessRows", () => {
  const agentA = identity({
    id: "agent://acme/sre/incident-triage-copilot",
    type: "agent",
    permissions: [permission("pagerduty_read", false, true), permission("pagerduty_admin", true, false)],
  });
  const mcpSanctioned = identity({
    id: "mcp://acme/sanctioned/pagerduty",
    type: "mcp_server",
    permissions: [permission("pagerduty_read")],
  });
  const notAnAgent = identity({ id: "user://acme/alice", type: "human" });

  it("builds one row per agent-typed identity only, skipping every other type", () => {
    const rows = buildAccessRows([agentA, mcpSanctioned, notAnAgent], [], null);
    expect(rows.map((r) => r.identity.id)).toEqual([agentA.id]);
  });

  it("sets policy to null when policies is null - never a fabricated all-zero overlay", () => {
    const rows = buildAccessRows([agentA], [], null);
    expect(rows[0].policy).toBeNull();
  });

  it("populates policy.matched/overlay when policies is provided", () => {
    const policies = [policy({ id: "p1", target: "agent://acme/sre/*", deny_tool: ["shell_exec"] })];
    const rows = buildAccessRows([agentA], [], policies);
    expect(rows[0].policy).not.toBeNull();
    expect(rows[0].policy?.matched.map((p) => p.id)).toEqual(["p1"]);
    expect(rows[0].policy?.overlay.deniedTools).toEqual(["shell_exec"]);
  });

  it("computes mcpReach from the SAME identities array, not the policies", () => {
    const rows = buildAccessRows([agentA, mcpSanctioned], [], null);
    expect(rows[0].mcpReach.sanctionedTools).toEqual(["pagerduty_read"]);
  });

  it("counts this identity's own agent_shadow_tool alerts, without deriving shadowTools from them", () => {
    const alerts = [
      alert({ detector: "agent_shadow_tool", identity: agentA.id, summary: "x" }),
      alert({ detector: "agent_shadow_tool", identity: agentA.id, summary: "y" }),
      alert({ detector: "agent_shadow_tool", identity: "agent://someone-else", summary: "z" }),
    ];
    const rows = buildAccessRows([agentA], alerts, null);
    expect(rows[0].agentShadowToolAlertCount).toBe(2);
    // No shadow servers were passed in at all, so the join-derived
    // shadowTools must stay empty regardless of the alert count above.
    expect(rows[0].mcpReach.shadowTools).toEqual([]);
  });
});

// ---------------------------------------------------------------------------
// Worst-first ordering (I5 spec: shadow desc, adminUnused desc, unused desc)
// ---------------------------------------------------------------------------

describe("sortAccessRowsWorstFirst", () => {
  it("orders shadow-tool count desc, then unused-admin count desc, then unused count desc", () => {
    const shadowServer = identity({
      id: "mcp://acme/shadow/x",
      type: "mcp_server",
      permissions: [permission("shadow_tool")],
    });

    const clean = identity({ id: "agent://acme/clean", permissions: [permission("p1", false, true)] });
    const someUnused = identity({
      id: "agent://acme/some-unused",
      permissions: [permission("p1", false, true), permission("p2", false, false)],
    });
    const adminUnused = identity({
      id: "agent://acme/admin-unused",
      permissions: [permission("p1", false, true), permission("p2", true, false)],
    });
    const shadowReach = identity({
      id: "agent://acme/shadow-reach",
      permissions: [permission("p1", false, true), permission("shadow_tool", false, true)],
    });
    // Shadow is alert-derived, not a property of the identity itself (idryx
    // exposes no shadow flag over REST) - without this alert, `shadowServer`
    // would be read as sanctioned and `shadowReach` would rank last, not
    // first.
    const alerts = [alert({ detector: "shadow_mcp", identity: shadowServer.id })];

    const rows = buildAccessRows([clean, someUnused, adminUnused, shadowReach, shadowServer], alerts, null);
    const sorted = sortAccessRowsWorstFirst(rows);
    expect(sorted.map((r) => r.identity.id)).toEqual([
      shadowReach.id,
      adminUnused.id,
      someUnused.id,
      clean.id,
    ]);
  });

  it("breaks a full tie deterministically by identity id, and never mutates the input array", () => {
    const a = identity({ id: "agent://acme/b" });
    const b = identity({ id: "agent://acme/a" });
    const rows = buildAccessRows([a, b], [], null);
    const original = [...rows];
    const sorted = sortAccessRowsWorstFirst(rows);
    expect(sorted.map((r) => r.identity.id)).toEqual(["agent://acme/a", "agent://acme/b"]);
    expect(rows).toEqual(original);
  });
});
