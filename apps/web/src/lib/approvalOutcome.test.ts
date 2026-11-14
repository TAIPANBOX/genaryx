import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import { ApprovalsInbox } from "../components/ApprovalsInbox";
import { ApprovalOutcomeBadge } from "../components/ApprovalOutcomeBadge";
import { EventDetailCard } from "../components/EventDetailCard";
import { EventRow } from "../components/EventRow";
import type { Decision, Approval } from "../policyTypes";
import type { UiEvent } from "../types";
import { approvalClosureLine, approvalOutcome } from "./approvalOutcome";
import { parseMailLink } from "./mailLink";

// `import.meta.glob` rather than `node:fs`: this app has no `@types/node`.
const SOURCES = import.meta.glob(["../components/ApprovalsInbox.tsx", "../components/Agent360.tsx", "../policyTypes.ts"], {
  query: "?raw",
  import: "default",
  eager: true,
}) as Record<string, string>;
const source = (name: string): string => {
  const hit = Object.entries(SOURCES).find(([path]) => path.endsWith(name));
  if (!hit) throw new Error(`no source for ${name}`);
  return hit[1];
};

const RED = "var(--sev-critical)";
const GREEN = "var(--sev-low)";

function closed(decision: string | null, decidedBy = "wardryx:hold-ttl"): Approval {
  return {
    approval_id: "ap_9",
    agent_id: "agent://acme.example/agents/bot-1",
    run_id: "run-9",
    requested_at: "2026-11-13T10:00:00Z",
    decided_at: "2026-11-14T10:00:00Z",
    decided_by: decidedBy,
    decision,
    pending: false,
    tool_names: ["ask"],
    est_cost_usd: null,
    reason: "no grant for ask",
    on_behalf_of: null,
    policy_version: "pv-1",
    org: "acme",
    model: null,
    tool_call: null,
    hold_kind: null,
    identity_source: null,
  };
}

function inbox(approvals: Approval[]): string {
  return renderToStaticMarkup(
    createElement(ApprovalsInbox, {
      approvals,
      policies: [],
      role: "admin",
      onDecide: async () => {},
      onAlwaysAllow: async () => {},
      grantedTokens: [],
      onDismissToken: () => {},
      focusApprovalId: null,
      mutedKeys: new Set<string>(),
      onToggleMuteAgent: () => {},
      onOpenAgent: () => {},
    }),
  );
}

function ev(type: string, data: unknown): UiEvent {
  return {
    id: 1,
    env: "prod",
    ts: "2026-11-14T10:00:00Z",
    source: "wardryx",
    type,
    agent_id: "agent://acme.example/agents/bot-1",
    run_id: "run-9",
    severity: type === "approval_expired" ? "medium" : "info",
    schema: "v1.0",
    on_behalf_of: [],
    data,
    prev_hash: null,
    raw: "{}",
    file: null,
    off: 0,
  };
}

const EXPIRED = "Expired, nobody decided in time";
const WITHDRAWN = "Withdrawn by the agent";

describe("how a closed hold reads", () => {
  it("an_expired_hold_reads_as_expired_and_nobody_decided_in_time", () => {
    const o = approvalOutcome({ pending: false, decision: "expired" });
    expect(o.kind).toBe("expired");
    expect(o.label).toBe(EXPIRED);
  });

  it("a_withdrawn_hold_reads_as_withdrawn_by_the_agent", () => {
    const o = approvalOutcome({ pending: false, decision: "withdrawn" });
    expect(o.kind).toBe("withdrawn");
    expect(o.label).toBe(WITHDRAWN);
  });

  it("expired_and_withdrawn_are_neither_the_refusal_red_nor_the_grant_green", () => {
    const e = approvalOutcome({ pending: false, decision: "expired" });
    const w = approvalOutcome({ pending: false, decision: "withdrawn" });
    expect(e.tone).not.toBe(RED);
    expect(e.tone).not.toBe(GREEN);
    expect(w.tone).toBe(e.tone);
  });

  it("a_grant_and_a_deny_read_as_before", () => {
    expect(approvalOutcome({ pending: false, decision: "grant" })).toMatchObject({ kind: "grant", label: "grant", tone: GREEN });
    expect(approvalOutcome({ pending: false, decision: "deny" })).toMatchObject({ kind: "deny", label: "deny", tone: RED });
    expect(approvalOutcome({ pending: true, decision: null })).toMatchObject({ kind: "pending", label: "pending" });
  });

  it("an_unknown_decision_reads_as_closed_with_its_value_and_is_neutral", () => {
    const o = approvalOutcome({ pending: false, decision: "revoked" });
    expect(o.kind).toBe("closed");
    expect(o.label).toBe("Closed (revoked)");
    expect(o.tone).not.toBe(RED);
    expect(o.tone).not.toBe(GREEN);
  });

  it("a_closed_hold_with_no_decision_is_neutral_and_never_a_refusal", () => {
    const o = approvalOutcome({ pending: false, decision: null });
    expect(o.tone).not.toBe(RED);
    expect(o.tone).not.toBe(GREEN);
    expect(o.label).toBe("Closed");
  });

  it("two_hundred_seeds_of_hostile_decision_values_never_throw_and_never_turn_red_or_green", () => {
    const alphabet = ["a", "Z", "grant", "deny", "\u0000", "‮", "\n", " ", "x".repeat(300), "\u{1f600}", "<b>", "&", "expired", "withdrawn"];
    let seed = 7;
    const next = () => {
      seed = (seed * 1103515245 + 12345) & 0x7fffffff;
      return seed;
    };
    for (let i = 0; i < 200; i++) {
      let value = "";
      const n = next() % 5;
      for (let j = 0; j < n; j++) value += alphabet[next() % alphabet.length];
      const wire: unknown = i % 7 === 0 ? { nested: value } : i % 11 === 0 ? next() : value;
      const o = approvalOutcome({ pending: false, decision: wire as string });
      const exact = value === "grant" || value === "deny";
      if (!exact || typeof wire !== "string") {
        expect(o.tone).not.toBe(RED);
        expect(o.tone).not.toBe(GREEN);
      }
      // eslint-disable-next-line no-control-regex
      expect(o.label).not.toMatch(/[\u0000-\u001f‮]/);
      expect(o.label.length).toBeLessThanOrEqual(120);
    }
  });
});

describe("where a closed hold is shown", () => {
  it("the_history_row_shows_an_expired_hold_neutral_with_the_system_as_decider", () => {
    const html = inbox([closed("expired")]);
    expect(html).toContain(EXPIRED);
    expect(html).toContain("wardryx:hold-ttl");
    expect(html).not.toContain(RED);
  });

  it("the_history_row_shows_a_withdrawn_hold_neutral_with_the_agent_as_decider", () => {
    const html = inbox([closed("withdrawn", "agent:agent://acme.example/agents/bot-1")]);
    expect(html).toContain(WITHDRAWN);
    expect(html).not.toContain(RED);
  });

  it("the_history_row_closes_an_unknown_value_as_closed_and_keeps_rendering", () => {
    const html = inbox([closed("revoked"), closed("deny"), closed("grant")]);
    expect(html).toContain("Closed (revoked)");
    expect(html.match(/--tone:var\(--sev-critical\)/g)?.length).toBe(1);
    expect(html.match(/--tone:var\(--sev-low\)/g)?.length).toBe(1);
  });

  it("the_badge_on_the_agent_card_and_the_one_in_the_inbox_are_one_component", () => {
    const badge = renderToStaticMarkup(createElement(ApprovalOutcomeBadge, { approval: { pending: false, decision: "expired" } }));
    expect(badge).toContain(EXPIRED);
    expect(badge).not.toContain(RED);
    const inboxSrc = source("ApprovalsInbox.tsx");
    const agentSrc = source("Agent360.tsx");
    expect(inboxSrc).toContain("<ApprovalOutcomeBadge");
    expect(agentSrc).toContain("<ApprovalOutcomeBadge");
    expect(agentSrc).not.toContain('a.decision === "grant"');
  });
});

describe("the event feed", () => {
  it("an_approval_expired_event_says_how_long_it_waited_and_the_limit", () => {
    const line = approvalClosureLine(
      ev("approval_expired", { approval_id: "ap_9", waited_seconds: 86400, ttl_seconds: 86400, decided_by: "wardryx:hold-ttl" }),
    );
    expect(line).toContain(EXPIRED);
    expect(line).toContain("waited 24h");
    expect(line).toContain("limit 24h");
  });

  it("an_approval_withdrawn_event_says_the_agent_withdrew_it_and_how_long_it_waited", () => {
    const line = approvalClosureLine(ev("approval_withdrawn", { approval_id: "ap_9", waited_seconds: 95, decided_by: "agent:x" }));
    expect(line).toContain(WITHDRAWN);
    expect(line).toContain("waited 1m 35s");
  });

  it("other_events_have_no_closure_line", () => {
    expect(approvalClosureLine(ev("approval_denied", { waited_seconds: 5 }))).toBeNull();
    expect(approvalClosureLine(ev("policy_deny", null))).toBeNull();
  });

  it("a_closure_line_survives_missing_hostile_and_negative_numbers", () => {
    for (const data of [null, undefined, "x", [], {}, { waited_seconds: "5" }, { waited_seconds: -1, ttl_seconds: Number.NaN }, { waited_seconds: 1e300, ttl_seconds: Infinity }]) {
      const line = approvalClosureLine(ev("approval_expired", data));
      expect(line).toContain(EXPIRED);
      expect(line).not.toMatch(/NaN|Infinity|-1/);
    }
  });

  it("the_expanded_event_row_and_the_detail_card_show_the_closure_line", () => {
    const e = ev("approval_expired", { approval_id: "ap_9", waited_seconds: 3600, ttl_seconds: 3600, decided_by: "wardryx:hold-ttl" });
    const row = renderToStaticMarkup(createElement(EventRow, { event: e, expanded: true, onToggle: () => {} }));
    const card = renderToStaticMarkup(createElement(EventDetailCard, { event: e, onClose: () => {} }));
    for (const html of [row, card]) {
      expect(html).toContain(EXPIRED);
      expect(html).toContain("waited 1h");
    }
    const plain = renderToStaticMarkup(createElement(EventRow, { event: ev("policy_allow", {}), expanded: true, onToggle: () => {} }));
    expect(plain).not.toContain("data-approval-closure");
  });

  it("the_mail_link_for_an_expired_or_withdrawn_hold_opens_the_policy_panel", () => {
    expect(parseMailLink("/i/approval_expired:ap_9")?.view).toBe("policy");
    expect(parseMailLink("/i/approval_withdrawn:ap_9")?.view).toBe("policy");
  });
});

describe("what a person can send", () => {
  it("the_decide_action_takes_only_grant_and_deny", () => {
    const ok: Decision[] = ["grant", "deny"];
    // @ts-expect-error a hold is closed as expired or withdrawn by wardryx and its agent, never sent by a person
    const expired: Decision = "expired";
    // @ts-expect-error same
    const withdrawn: Decision = "withdrawn";
    expect(ok).toHaveLength(2);
    expect([expired, withdrawn]).toHaveLength(2);
    expect(source("policyTypes.ts")).toContain('export type Decision = "grant" | "deny";');
  });

  it("no_control_in_the_inbox_cancels_or_closes_a_hold", () => {
    const pending: Approval = { ...closed(null), pending: true, decided_at: null, decided_by: null, hold_kind: "no_grant" };
    for (const role of ["viewer", "approver", "admin"] as const) {
      const html = renderToStaticMarkup(
        createElement(ApprovalsInbox, {
          approvals: [pending, closed("expired")],
          policies: [],
          role,
          onDecide: async () => {},
          onAlwaysAllow: async () => {},
          grantedTokens: [],
          onDismissToken: () => {},
          focusApprovalId: null,
          mutedKeys: new Set<string>(),
          onToggleMuteAgent: () => {},
          onOpenAgent: () => {},
        }),
      );
      const buttons = [...html.matchAll(/<button[^>]*>([^<]*)<\/button>/g)].map((m) => m[1].toLowerCase());
      for (const label of buttons) expect(label).not.toMatch(/cancel|withdraw|expire|close hold|revoke/);
    }
  });
});
