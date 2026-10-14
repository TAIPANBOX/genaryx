import { createElement, type ReactNode } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import { insetOfText } from "../testing/cardInset";
import type { Approval } from "../policyTypes";
import type { IdryxAlert, IdryxIdentity } from "../identityTypes";
import type { EngramStats } from "../memoryTypes";
import type { VerdryxRunSummary } from "../qualityTypes";
import { AnomaliesView } from "./AnomaliesView";
import { ApprovalsInbox } from "./ApprovalsInbox";
import { BusCoverageFootnote } from "./BusCoverageFootnote";
import { IdentityAlerts } from "./IdentityAlerts";
import { IdentityList } from "./IdentityList";
import { MemoryStats } from "./MemoryStats";
import { RunsToolbar } from "./MoneyView";
import { PolicyList } from "./PolicyList";
import { QualityRunDetail } from "./QualityRunDetail";
import { Section } from "./dash";

/**
 * Every dashboard card body insets its own rows by 20px (the `.d-sechead` title
 * already sits on 20px). A row that starts at the card's own edge reads as
 * unfinished, and the browser scan found six of them. Each test renders the
 * component inside a real `Section` and asserts the inset of the rows it owns.
 */
const INSET = 20;
const noop = () => {};
const inCard = (node: ReactNode) => renderToStaticMarkup(createElement(Section, { title: "Card", children: node }));

const COVERAGE = { read: 500, limit: 500, truncated: false, planes: ["tokenfuse", "idryx"], incidentRows: 5 };

const IDENTITY: IdryxIdentity = {
  id: "agent-forge-7",
  type: "agent",
  privileged: false,
  source: "forge",
  owner: "ops",
  created: "2026-10-01T00:00:00Z",
  last_used: "2026-10-14T09:00:00Z",
  runtime: "python",
  on_behalf_of: [],
  permissions: [],
  remediation: null,
  rotation: null,
  events: 4,
  alerts: 1,
};

const ALERT: IdryxAlert = {
  detector: "stale_key",
  identity: "agent-forge-7",
  severity: "critical",
  time: "2026-10-14T09:00:00Z",
  summary: "key unused for 90 days",
};

const APPROVAL: Approval = {
  approval_id: "appr-1",
  agent_id: "agent-forge-7",
  run_id: "run-1",
  requested_at: "2026-10-14T09:00:00Z",
  decided_at: null,
  decided_by: null,
  decision: null,
  pending: true,
  tool_names: ["send_payment"],
  est_cost_usd: 12.5,
  reason: "over the unit cap",
  on_behalf_of: null,
  policy_version: "v7",
  org: "forge",
  model: "claude-haiku-5-5",
};

const DECIDED: Approval = { ...APPROVAL, approval_id: "appr-2", pending: false, decided_at: "2026-10-14T10:00:00Z", decided_by: "ops", decision: "allow" };

const APPROVALS_PROPS = {
  onDecide: async () => {},
  grantedToken: null,
  onDismissToken: noop,
  focusApprovalId: null,
  mutedKeys: new Set<string>(),
  onToggleMuteAgent: noop,
  onOpenAgent: noop,
};

const STATS: EngramStats = {
  agent_id: null,
  counts: { episodic: 4, semantic: 2, procedural: 0 },
  vector_index_size: 12,
  facts_total: 9,
  facts_active: 7,
  facts_superseded: 2,
  entities: 3,
  reflections: 1,
  db_path: "/var/lib/engram/engram.db",
  db_size_bytes: 4096,
};

const RUN: VerdryxRunSummary = {
  run: { id: "run-1", model: "claude-haiku-5-5", started_at: "2026-10-14T09:00:00Z", finished_at: null },
  case_count: 3,
  mean_score: 0.812,
  total_tokens: 1200,
  total_cost_usd: 0.04,
  unanswered_count: 0,
  unanswered_by_reason: [],
  unanswered_supported: false,
};

describe("Overview: the Incident center's bus footnote", () => {
  it("the_bus_footnote_of_the_incident_center_sits_on_the_20px_inset", () => {
    const html = inCard(createElement(BusCoverageFootnote, { coverage: COVERAGE }));
    expect(insetOfText(html, "bus event(s) are incidents")).toBe(INSET);
  });
});

describe("Incidents view: the footnote and the severity chips", () => {
  const html = renderToStaticMarkup(createElement(AnomaliesView, { onSelectView: noop, onOpenIncident: noop }));

  it("the_incidents_footnote_sits_on_the_20px_inset", () => {
    expect(insetOfText(html, "no events read off the bus")).toBe(INSET);
  });

  it("the_incidents_severity_chips_sit_on_the_20px_inset", () => {
    expect(insetOfText(html, "critical")).toBe(INSET);
  });
});

describe("Money: the Runs card's sort row", () => {
  it("the_runs_sort_row_sits_on_the_20px_inset", () => {
    const html = inCard(
      createElement(RunsToolbar, {
        sort: { key: "spend", dir: "desc" },
        onSort: noop,
        exportButtons: createElement("button", { type: "button" }, "Export CSV"),
      }),
    );
    expect(insetOfText(html, "sort")).toBe(INSET);
  });
});

describe("Identity: the identities table's type filter and note", () => {
  it("the_identity_type_filter_and_note_sit_on_the_20px_inset", () => {
    const html = inCard(createElement(IdentityList, { identities: [IDENTITY], onOpenAgent: noop }));
    expect(insetOfText(html, "A dash is a field idryx sent empty")).toBe(INSET);
    expect(insetOfText(html, "human")).toBe(INSET);
  });

  it("an_empty_identity_list_says_so_on_the_20px_inset", () => {
    const html = inCard(createElement(IdentityList, { identities: [], onOpenAgent: noop }));
    expect(insetOfText(html, "no identities in this snapshot.")).toBe(INSET);
  });
});

describe("Identity: the Alerts card's severity filter", () => {
  it("the_identity_alerts_severity_filter_sits_on_the_20px_inset", () => {
    const html = inCard(createElement(IdentityAlerts, { alerts: [ALERT], onOpenAgent: noop }));
    expect(insetOfText(html, "critical")).toBe(INSET);
  });

  it("an_empty_identity_alerts_list_says_so_on_the_20px_inset", () => {
    const html = inCard(createElement(IdentityAlerts, { alerts: [], onOpenAgent: noop }));
    expect(insetOfText(html, "no alerts in this snapshot.")).toBe(INSET);
  });
});

describe("Policy: the Policies card's version chip", () => {
  it("the_policy_version_chip_sits_on_the_20px_inset", () => {
    const html = inCard(createElement(PolicyList, { policies: [], policyVersion: "v7" }));
    expect(insetOfText(html, "policy_version")).toBe(INSET);
  });

  it("an_empty_policy_list_says_so_on_the_20px_inset", () => {
    const html = inCard(createElement(PolicyList, { policies: [], policyVersion: "v7" }));
    expect(insetOfText(html, "no policies configured.")).toBe(INSET);
  });
});

describe("Policy: the Approvals Inbox rows", () => {
  it("a_pending_approval_box_sits_on_the_20px_inset", () => {
    const html = inCard(createElement(ApprovalsInbox, { ...APPROVALS_PROPS, approvals: [APPROVAL] }));
    expect(insetOfText(html, "over the unit cap", "panel")).toBe(INSET);
  });

  it("the_approvals_history_label_sits_on_the_20px_inset", () => {
    const html = inCard(createElement(ApprovalsInbox, { ...APPROVALS_PROPS, approvals: [APPROVAL, DECIDED] }));
    expect(insetOfText(html, "History")).toBe(INSET);
  });

  it("an_empty_approvals_inbox_says_so_on_the_20px_inset", () => {
    const html = inCard(createElement(ApprovalsInbox, { ...APPROVALS_PROPS, approvals: [] }));
    expect(insetOfText(html, "no pending approvals.")).toBe(INSET);
  });
});

describe("Memory: the Store card's stat tiles and scope line", () => {
  const html = inCard(createElement(MemoryStats, { stats: STATS }));

  it("the_memory_store_scope_line_sits_on_the_20px_inset", () => {
    expect(insetOfText(html, "scope:")).toBe(INSET);
  });

  it("the_memory_store_stat_tiles_sit_on_the_20px_inset_in_every_grid", () => {
    expect(insetOfText(html, "Episodic", "panel")).toBe(INSET);
    expect(insetOfText(html, "Facts active", "panel")).toBe(INSET);
    expect(insetOfText(html, "Vector index size", "panel")).toBe(INSET);
  });
});

describe("Quality: the Run Detail card", () => {
  it("the_quality_run_detail_tiles_sit_on_the_20px_inset", () => {
    const html = inCard(createElement(QualityRunDetail, { summary: RUN, scores: [], error: null }));
    expect(insetOfText(html, "Mean score", "panel")).toBe(INSET);
  });

  it("the_quality_error_box_sits_on_the_20px_inset", () => {
    const html = inCard(
      createElement(QualityRunDetail, { summary: RUN, scores: [], error: { kind: "verdryx", message: "boom" } }),
    );
    expect(insetOfText(html, "boom", "d-card")).toBe(INSET);
  });

  it("an_empty_quality_run_detail_says_so_on_the_20px_inset", () => {
    const html = inCard(createElement(QualityRunDetail, { summary: null, scores: [], error: null }));
    expect(insetOfText(html, "select a run above to see its detail.")).toBe(INSET);
  });
});
