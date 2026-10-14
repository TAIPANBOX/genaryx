import { describe, expect, it } from "vitest";

import { agentFeedLine, claimSentence } from "./attribution";
import { aggregateIncidents, incidentSubject, INCIDENT_BANDS, TAB_BANDS } from "./incidents";
import { overviewPiiLabel, runDecisionSentence, runSignals } from "./runStatus";
import { runsExportRows } from "./moneyExport";
import type { UiEvent } from "../types";
import type { Overview, Run } from "../moneyTypes";

/**
 * Two optional event shapes from TokenFuse (2026-10-14): a call refused for
 * its client key, written as `identity_mismatch` with `data.reason`
 * `missing_key` or `unknown_key` and a `sha256:` fingerprint, never the key
 * (tokenfuse#432); and a PII finding the gateway forwarded or masked,
 * `dlp_shadow` at medium with kinds and counts only (tokenfuse#433). Neither
 * is emitted by a gateway older than those changes, and nothing here waits
 * for one.
 */
const CLAIMED = "agent://lab.taipanbox.dev/w2/capped";

function event(over: Partial<UiEvent>): UiEvent {
  return {
    id: 1,
    env: "live",
    ts: "2026-10-14T13:30:00Z",
    source: "tokenfuse",
    type: "identity_mismatch",
    agent_id: CLAIMED,
    run_id: null,
    severity: "high",
    schema: "taipanbox.dev/agent-event/v0.2",
    on_behalf_of: [],
    data: {},
    prev_hash: null,
    raw: "",
    file: null,
    off: null,
    ...over,
  } as UiEvent;
}

const missing = (id: number) =>
  event({ id, data: { reason: "missing_key", key_fingerprint: null, agent_id: CLAIMED, unreported_since_last: 4 } });
const unknown = (id: number, fp: string) =>
  event({ id, data: { reason: "unknown_key", key_fingerprint: fp, agent_id: CLAIMED, unreported_since_last: 0 } });

const rows = (events: UiEvent[], bands = INCIDENT_BANDS) =>
  aggregateIncidents({ moneyIncidents: [], identityAlerts: [], busEvents: events, postureFindings: [] }, { bands });

describe("a call refused for its client key", () => {
  it("a_refused_client_key_reads_as_a_refused_key_not_an_identity_mismatch", () => {
    const [row] = rows([missing(1), missing(2)]);
    expect(row.title).toBe("client key refused: none presented");
    expect(row.detail).toContain(`claimed ${CLAIMED} with no client key, refused before the call`);
    expect(row.detail).toContain("4 more held back by the rate limit");
    expect(row.occurrences).toBe(2);
    // Filed like any refusal with no key id, never under the claimed agent.
    expect(incidentSubject(row)).toBe("key:(none)");
  });

  it("two_unknown_keys_are_two_incidents_and_a_missing_key_a_third", () => {
    const out = rows([unknown(1, "sha256:0123456789abcdef"), unknown(2, "sha256:fedcba9876543210"), missing(3)]);
    expect(out).toHaveLength(3);
    const titles = out.map((r) => r.title).sort();
    expect(titles).toEqual(["client key refused: none presented", "client key refused: unknown key", "client key refused: unknown key"]);
    expect(out.some((r) => r.detail.includes("an unknown key sha256:0123456789abcdef"))).toBe(true);
  });

  it("a real identity mismatch keeps its own words", () => {
    const real = event({ data: { key_id: "forge-imposter", agent_id: CLAIMED, reason: "agent_id_not_allowed" } });
    const [row] = rows([real]);
    expect(row.title).toBe("identity mismatch");
    expect(claimSentence(real)).toBe(`claimed ${CLAIMED} with key forge-imposter`);
  });

  it("never prints a presented value that is not a fingerprint", () => {
    const leaked = unknown(1, "sk-live-THIS-IS-A-KEY");
    expect(claimSentence(leaked)).toBe(`claimed ${CLAIMED} with an unknown key, refused before the call`);
    expect(JSON.stringify(rows([leaked]).map((r) => [r.title, r.detail]))).not.toContain("sk-live");
  });

  it("reads on the claimed agent's own feed as a claim, not its activity", () => {
    expect(agentFeedLine(missing(1), CLAIMED)).toEqual({
      text: "identity_mismatch: claimed with no client key, refused",
      own: false,
    });
  });
});

describe("personal data the gateway found and let through", () => {
  const shadow = (counts: Record<string, unknown>, mode = "shadow") =>
    event({
      type: "dlp_shadow",
      severity: "medium",
      run_id: "w2-b6",
      data: { summary: "PII found in the request", counts, mode, unit: "lab" },
    });

  it("a_pii_finding_in_shadow_reaches_the_anomalies_tab_with_its_kinds", () => {
    const [row] = rows([shadow({ pii_card: 1, pii_email: 2 })], TAB_BANDS);
    expect(row.title).toBe("personal data found (shadow)");
    expect(row.detail).toContain("pii_card 1, pii_email 2");
    expect(row.detail).toContain("unit lab");
    // Medium: the Anomalies tab, not the fire card.
    expect(rows([shadow({ pii_card: 1 })])).toHaveLength(0);
    expect(rows([shadow({ pii_card: 1 }, "mask")], TAB_BANDS)[0].title).toBe("personal data masked");
  });

  it("keeps only numeric counts and bounds what it prints", () => {
    const [row] = rows([shadow({ pii_card: "x", ["k".repeat(80)]: 3, pii_phone: 1 })], TAB_BANDS);
    expect(row.detail).not.toContain("pii_card");
    expect(row.detail).toContain(`${"k".repeat(32)} 3`);
    expect(row.detail).not.toContain("k".repeat(33));
  });
});

describe("the PII counts the Cloud reports", () => {
  const RUN = { run_id: "w2-b6", model: "m", unit: "", owner: "", agent_id: "", spent_usd: 0, budget_usd: null, budgets_read: true, calls: 3, cache_hits: 0, steps: 3, last_seen: "", killed: false } as Run;

  it("a_run_that_carried_personal_data_carries_a_pii_pill", () => {
    const pill = runSignals({ ...RUN, pii_calls: 3 }).find((s) => s.key === "pii");
    expect(pill?.label).toBe("pii ×3");
    expect(runSignals({ ...RUN, pii_calls: 0 })).toEqual([]);
    expect(runSignals(RUN)).toEqual([]);
    expect(runDecisionSentence({ ...RUN, blocked: 0, last_decision: "allow", pii_calls: 3 })).toContain("3 carried personal data");
    expect(runsExportRows([{ ...RUN, pii_calls: 3 }])[0].pii_calls).toBe(3);
    expect(runsExportRows([RUN])[0].pii_calls).toBeNull();
  });

  it("the_overview_says_how_many_calls_carried_personal_data_or_nothing_when_unreported", () => {
    const o = { total_calls: 91 } as Overview;
    expect(overviewPiiLabel(o)).toBeNull();
    expect(overviewPiiLabel({ ...o, pii_calls: 0 })).toBe("no personal data found");
    expect(
      overviewPiiLabel({ ...o, pii_calls: 4, pii_kinds: [{ kind: "pii_email", calls: 3 }, { kind: "pii_card", calls: 1 }] }),
    ).toBe("4 with personal data: email 3, card 1");
  });
});
