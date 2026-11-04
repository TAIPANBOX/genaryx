import { describe, expect, it } from "vitest";

import { mockInvoke } from "./mockPreview";
import { digestOk } from "./caseDigest";
import type { CaseFile } from "../caseTypes";
import type { UiEvent } from "../types";

/**
 * The published demo runs on the mock transport, so what it shows of a case is
 * what a visitor learns of the incident-to-proven-fix join: every question has
 * a line, a refusal is filed under its key, and a plane that does not answer
 * is a line and not a hole. Every row is one the demo itself serves.
 */
describe("the demo case", () => {
  const demoRefusalId = async (): Promise<number> => {
    const events = await mockInvoke<UiEvent[]>("recent_events", { limit: 500 });
    const refusal = events.find(
      (e) => e.type === "identity_mismatch" && (e.data as Record<string, unknown> | null)?.key_id === "forge-imposter",
    );
    expect(refusal).toBeDefined();
    return refusal!.id;
  };

  it("the_demo_assembles_a_case_for_an_identity_refusal_on_invented_rows", async () => {
    const id = await demoRefusalId();
    const events = await mockInvoke<UiEvent[]>("recent_events", { limit: 500 });
    const refusal = events.find((e) => e.id === id)!;
    const c = await mockInvoke<CaseFile>("case_file", { incident_id: `bus:${id}` });

    // Filed under the key, with the agent it claimed kept as a claim.
    expect(c.subject).toBe("key:forge-imposter");
    expect(c.claimed_agent).toBe(refusal.agent_id);
    expect(c.run_id).toBe(refusal.run_id);

    // The questions about an agent are not asked, and the case says why.
    for (const section of ["identity", "blast_radius"]) {
      const l = c.lines.find((x) => x.section === section)!;
      expect(l.state).toBe("not_asked");
      expect(l.reason).toContain("filed under a key, not an agent");
    }
    expect(c.facts.some((f) => f.section === "identity" || f.section === "blast_radius")).toBe(false);

    // One line per question, in the real order.
    expect(c.lines.map((l) => l.section)).toEqual([
      "event",
      "run_timeline",
      "spend",
      "identity",
      "blast_radius",
      "policy",
      "decisions",
      "similar_incidents",
      "prior_rulings",
    ]);

    // Every row is a row the demo transport serves, with a real digest.
    const eventFact = c.facts.find((f) => f.section === "event")!;
    expect(eventFact.row).toEqual(refusal);
    for (const f of c.facts.filter((x) => x.section === "run_timeline")) {
      expect(events.some((e) => String(e.id) === f.row_id && e.run_id === refusal.run_id)).toBe(true);
    }
    for (const f of c.facts) expect(digestOk(f)).toBe(true);
    expect(c.digest).toMatch(/^sha256:[0-9a-f]{64}$/);
  });

  it("the_demo_case_includes_a_plane_that_did_not_answer", async () => {
    const id = await demoRefusalId();
    const c = await mockInvoke<CaseFile>("case_file", { incident_id: `bus:${id}` });
    const failed = c.lines.filter((l) => l.state === "unavailable");
    expect(failed.length).toBeGreaterThanOrEqual(1);
    for (const l of failed) {
      expect(l.facts).toBe(0);
      expect(l.reason).toBe("the plane did not answer");
      expect(l.upstream?.trust).toBe("data");
      expect(c.facts.some((f) => f.section === l.section)).toBe(false);
    }
  });

  it("the_demo_refuses_an_incident_with_no_event_row_by_name", async () => {
    await expect(mockInvoke("case_file", { incident_id: "money:inc-1" })).rejects.toEqual({
      kind: "unsupported_source",
      source: "money",
    });
    await expect(mockInvoke("case_file", { incident_id: "bus:999999999" })).rejects.toEqual({
      kind: "not_found",
      event_id: 999999999,
    });
  });

  it("the_demo_felyx_reading_cites_only_ids_the_demo_case_holds", async () => {
    const id = await demoRefusalId();
    const incident = `bus:${id}`;
    const c = await mockInvoke<CaseFile>("case_file", { incident_id: incident });
    const answer = await mockInvoke<{ text: string; tool_trace: { name: string }[]; proposals: unknown[] }>(
      "copilot_explain_case",
      { incident_id: incident },
    );

    // Read from the case alone: no tool but the case, and nothing proposed.
    expect(answer.tool_trace.map((t) => t.name)).toEqual(["case_file"]);
    expect(answer.proposals).toEqual([]);

    // Every id in backticks is one the case holds.
    const held = new Set<string>([
      c.incident_id,
      c.subject,
      ...(c.claimed_agent ? [c.claimed_agent] : []),
      ...(c.run_id ? [c.run_id] : []),
      ...c.facts.map((f) => f.row_id),
    ]);
    const cited = [...answer.text.matchAll(/`([^`]+)`/g)].map((m) => m[1]);
    expect(cited.length).toBeGreaterThan(0);
    for (const token of cited) expect(held.has(token)).toBe(true);

    // It says what is missing, and the key is the subject, not the claim.
    expect(answer.text).toContain("Missing from the case");
    expect(answer.text).toContain(`\`${c.subject}\``);
  });

  it("the_demo_refuses_to_read_an_incident_with_no_event_row", async () => {
    await expect(mockInvoke("copilot_explain_case", { incident_id: "money:inc-1" })).rejects.toBe(
      "No case: a money incident has no event row to start from.",
    );
    await expect(mockInvoke("copilot_explain_case", { incident_id: "bus:999999999" })).rejects.toBe(
      "No case: the console's store holds no event row 999999999.",
    );
  });
});
