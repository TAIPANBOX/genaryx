/**
 * What the Crypto, Quality and Routines panels are allowed to say.
 *
 * Two things are under test and they are different sizes. The small one is
 * whether a field on the wire reaches the screen. The large one is whether an
 * ABSENT field is reported as absent: `0` and "not recorded" are different
 * statements, and every table here has at least one column where the wire can
 * carry either.
 *
 * The fixtures are the exact JSON shapes `crates/connectors/src/qryx.rs`'s own
 * tests parse (`ncsc_report_parses_all_three_milestones`,
 * `evidence_report_typed_digest_and_optional_signature`), so a test here that
 * passes is a test about a document qryx really emits.
 */
import { describe, expect, it } from "vitest";
import {
  BASELINE_EXPORT_COLUMNS,
  baselineExportMeta,
  baselineExportRows,
  CBOM_EXPORT_COLUMNS,
  cbomComponents,
  cbomExportMeta,
  cbomExportRows,
  evidenceProvenance,
  evidenceSeverityNote,
  evidenceSeverityRows,
  FINDING_EXPORT_COLUMNS,
  findingExportMeta,
  findingExportRows,
  milestoneViews,
  ncscProvenance,
  QUALITY_RUN_EXPORT_COLUMNS,
  qualityRunExportMeta,
  qualityRunExportRows,
  ROUTINE_HISTORY_EXPORT_COLUMNS,
  routineHistoryExportMeta,
  routineHistoryExportRows,
} from "./cryptoExport";
import { toCsv } from "./download";
import type { EvidenceReport, NcscFinding, NcscReport } from "../cryptoTypes";
import type { VerdryxBaseline, VerdryxRunSummary } from "../qualityTypes";
import type { RoutineRunDto, RoutinesHistoryDto } from "../routinesTypes";

// ============================================================================
// Fixtures - the shapes the Rust connector's own tests parse
// ============================================================================

function finding(over: Partial<NcscFinding> = {}): NcscFinding {
  return {
    algorithm: "RSA-2048",
    type: "public-key",
    severity: "high",
    occurrences: 2,
    locations: ["a.go:10", "b.go:20"],
    externallyFacing: true,
    longLivedData: false,
    planned: false,
    ...over,
  };
}

/** `qryx scan --format ncsc`, the shape
 * `ncsc_report_parses_all_three_milestones` asserts on. Note 2031: count 1,
 * findings []. That combination is in the connector's own fixture. */
function ncscReport(over: Partial<NcscReport> = {}): NcscReport {
  return {
    standard: "NCSC PQC migration timeline (2028/2031/2035)",
    generatedAt: "2026-07-17T10:00:00Z",
    root: "/repo",
    discovery2028: {
      verdict: "at-risk",
      coverageBySource: { code: 3, certs: 1 },
      totalInventoried: 4,
      quantumVulnerableCount: 2,
      migrationPlanExists: false,
      migrationPlanNote: "no plan artifact",
      quantumVulnerableFindings: [finding()],
    },
    highestPriority2031: {
      verdict: "not-started",
      criteria: "quantum-vulnerable AND (externally-facing OR long-lived)",
      count: 1,
      migratedCount: 0,
      remainingCount: 1,
      note: "...",
      findings: [],
    },
    fullMigration2035: { verdict: "not-started", count: 2, findings: [] },
    ...over,
  };
}

function evidenceReport(over: Partial<EvidenceReport> = {}): EvidenceReport {
  return {
    tool: "qryx",
    version: "0.4.0",
    standard: "CNSA 2.0",
    generatedAt: "2026-07-17T10:00:00Z",
    root: "/repo",
    summary: {
      compliant: 8,
      nonCompliant: 2,
      issues: 2,
      total: 10,
      scorePct: 80,
      bySeverity: { high: 1, medium: 1 },
    },
    assets: [{ algorithm: "RSA-2048", compliant: false }],
    digest: "sha256:abcdef",
    signature: { alg: "ml-dsa-65", value: "BASE64SIG", publicKey: "BASE64SPKI" },
    ...over,
  };
}

function runSummary(over: Partial<VerdryxRunSummary> = {}): VerdryxRunSummary {
  return {
    run: { id: "run-1", model: "opus", started_at: "2026-08-01T09:00:00Z", finished_at: "2026-08-01T09:30:00Z" },
    case_count: 12,
    mean_score: 0.8125,
    total_tokens: 4200,
    total_cost_usd: 1.5,
    ...over,
  };
}

function routineRun(over: Partial<RoutineRunDto> = {}): RoutineRunDto {
  return {
    schema: "stackup.routine-run/v1",
    routine: "qryx-trend",
    started_at: "2026-08-20T02:00:00Z",
    finished_at: "2026-08-20T02:00:41Z",
    exit_code: 0,
    status: "ok",
    reason: null,
    artifact: "out/qryx-trend-2026-08-20.json",
    summary: "4 inventoried, 2 quantum-vulnerable",
    ...over,
  };
}

const TAKEN_AT = "2026-08-26T12:00:00.000Z";
const ENV = "console.example";

function caveatsOf(meta: { caveats?: string[] }): string {
  return (meta.caveats ?? []).join("\n");
}

// ============================================================================
// 1. The list behind the count
// ============================================================================

describe("milestoneViews", () => {
  it("carries all three milestones, not only the one the panel showed", () => {
    const keys = milestoneViews(ncscReport()).map((m) => m.key);
    expect(keys).toEqual(["discovery2028", "highestPriority2031", "fullMigration2035"]);
  });

  it("hands each milestone its OWN findings, from its own field", () => {
    const report = ncscReport({
      highestPriority2031: {
        verdict: "at-risk",
        criteria: "externally-facing",
        count: 1,
        migratedCount: 0,
        remainingCount: 1,
        note: "",
        findings: [finding({ algorithm: "ECDSA-P256", type: "certificate" })],
      },
      fullMigration2035: {
        verdict: "not-started",
        count: 2,
        findings: [finding({ algorithm: "DH-2048" }), finding({ algorithm: "RSA-4096" })],
      },
    });
    const by = Object.fromEntries(milestoneViews(report).map((m) => [m.key, m.findings.map((f) => f.algorithm)]));
    expect(by.discovery2028).toEqual(["RSA-2048"]);
    expect(by.highestPriority2031).toEqual(["ECDSA-P256"]);
    expect(by.fullMigration2035).toEqual(["DH-2048", "RSA-4096"]);
  });

  it("keeps each milestone's count as qryx reported it, never as findings.length", () => {
    // The connector's own fixture: 2028 counts 2 quantum-vulnerable from ONE
    // finding with occurrences 2, and 2035 counts 2 while carrying no list.
    const views = milestoneViews(ncscReport());
    expect(views.map((m) => m.count)).toEqual([2, 1, 2]);
    expect(views[0].findings.length).toBe(1);
  });

  it("says a missing list is missing, and does not call it an empty result", () => {
    // 2031 in the connector's fixture: count 1, findings []. An operator
    // reading "no findings" there would conclude the milestone is clear.
    const p = milestoneViews(ncscReport()).find((m) => m.key === "highestPriority2031");
    expect(p?.emptyNote).toBeTruthy();
    expect(p?.emptyNote).toMatch(/1/);
    expect(p?.emptyNote?.toLowerCase()).toContain("no finding list");
  });

  it("says nothing-in-scope when the count is genuinely zero", () => {
    const report = ncscReport({ fullMigration2035: { verdict: "on-track", count: 0, findings: [] } });
    const f = milestoneViews(report).find((m) => m.key === "fullMigration2035");
    expect(f?.emptyNote).toBeTruthy();
    expect(f?.emptyNote?.toLowerCase()).not.toContain("no finding list");
  });

  it("leaves emptyNote null when there is a list to show", () => {
    expect(milestoneViews(ncscReport())[0].emptyNote).toBeNull();
  });
});

// ============================================================================
// 2. Provenance that is on the wire and was not on the screen
// ============================================================================

describe("ncscProvenance", () => {
  it("reports the standard, the generation time and the scanned root", () => {
    const lines = ncscProvenance(ncscReport());
    const values = lines.map((l) => l.value);
    expect(values).toContain("NCSC PQC migration timeline (2028/2031/2035)");
    expect(values).toContain("2026-07-17T10:00:00Z");
    expect(values).toContain("/repo");
    expect(lines.every((l) => !l.missing)).toBe(true);
  });

  it("calls an empty generatedAt not recorded, never today's date", () => {
    const lines = ncscProvenance(ncscReport({ generatedAt: "" }));
    const generated = lines.find((l) => l.label.toLowerCase().includes("generated"));
    expect(generated?.missing).toBe(true);
    expect(generated?.value.toLowerCase()).toContain("not recorded");
  });
});

describe("evidenceProvenance", () => {
  it("names the tool, its version, the standard, the root and the generation time", () => {
    const values = evidenceProvenance(evidenceReport()).map((l) => l.value);
    expect(values).toContain("qryx 0.4.0");
    expect(values).toContain("CNSA 2.0");
    expect(values).toContain("/repo");
    expect(values).toContain("2026-07-17T10:00:00Z");
  });

  it("marks an unrecorded root as unrecorded", () => {
    const root = evidenceProvenance(evidenceReport({ root: "" })).find((l) => l.label.toLowerCase().includes("root"));
    expect(root?.missing).toBe(true);
  });
});

// ============================================================================
// 3. The evidence summary's total and its by-severity breakdown
// ============================================================================

describe("evidenceSeverityRows", () => {
  it("returns the breakdown the report carried, worst severity first", () => {
    const rows = evidenceSeverityRows(
      evidenceReport({
        summary: { compliant: 5, nonCompliant: 5, issues: 5, total: 10, scorePct: 50, bySeverity: { low: 1, critical: 2, high: 2 } },
      }),
    );
    expect(rows.map((r) => r.severity)).toEqual(["critical", "high", "low"]);
    expect(rows.map((r) => r.count)).toEqual([2, 2, 1]);
  });

  it("keeps a severity it does not recognise rather than dropping it", () => {
    const rows = evidenceSeverityRows(
      evidenceReport({
        summary: { compliant: 0, nonCompliant: 1, issues: 1, total: 1, scorePct: 0, bySeverity: { moderate: 1 } },
      }),
    );
    expect(rows.map((r) => r.severity)).toEqual(["moderate"]);
  });

  it("returns no rows when the report carried no breakdown", () => {
    const rows = evidenceSeverityRows(
      evidenceReport({
        summary: { compliant: 8, nonCompliant: 2, issues: 2, total: 10, scorePct: 80, bySeverity: {} },
      }),
    );
    expect(rows).toEqual([]);
  });
});

describe("evidenceSeverityNote", () => {
  it("is silent when there is a breakdown to render", () => {
    expect(evidenceSeverityNote(evidenceReport())).toBeNull();
  });

  it("says the non-compliant assets are unattributed when the breakdown is missing", () => {
    // The dangerous case: 2 non-compliant and no severities. Rendering that as
    // an empty severity table reads as "nothing severe".
    const note = evidenceSeverityNote(
      evidenceReport({
        summary: { compliant: 8, nonCompliant: 2, issues: 2, total: 10, scorePct: 80, bySeverity: {} },
      }),
    );
    expect(note).toBeTruthy();
    expect(note).toMatch(/2/);
  });

  it("says so plainly when there is nothing non-compliant to break down", () => {
    const note = evidenceSeverityNote(
      evidenceReport({
        summary: { compliant: 10, nonCompliant: 0, issues: 0, total: 10, scorePct: 100, bySeverity: {} },
      }),
    );
    expect(note).toBeTruthy();
    expect(note).not.toMatch(/unattributed/);
  });
});

// ============================================================================
// 4. Exports
// ============================================================================

describe("findingExportRows", () => {
  it("exports every milestone's findings, each labelled with its milestone", () => {
    const report = ncscReport({
      fullMigration2035: { verdict: "not-started", count: 2, findings: [finding({ algorithm: "DH-2048" })] },
    });
    const rows = findingExportRows(report);
    expect(rows).toHaveLength(2);
    expect(rows.map((r) => r.algorithm)).toEqual(["RSA-2048", "DH-2048"]);
    expect(new Set(rows.map((r) => r.milestone)).size).toBe(2);
  });

  it("leaves locations unrecorded rather than writing an empty-looking value", () => {
    const report = ncscReport({
      discovery2028: {
        verdict: "at-risk",
        coverageBySource: {},
        totalInventoried: 1,
        quantumVulnerableCount: 1,
        migrationPlanExists: false,
        migrationPlanNote: "",
        quantumVulnerableFindings: [finding({ locations: [] })],
      },
    });
    expect(findingExportRows(report)[0].locations).toBeNull();
  });

  it("writes an unrecorded location as an EMPTY csv cell, never a dash or a zero", () => {
    const report = ncscReport({
      discovery2028: {
        verdict: "at-risk",
        coverageBySource: {},
        totalInventoried: 1,
        quantumVulnerableCount: 1,
        migrationPlanExists: false,
        migrationPlanNote: "",
        quantumVulnerableFindings: [finding({ locations: [] })],
      },
      highestPriority2031: { verdict: "on-track", criteria: "", count: 0, migratedCount: 0, remainingCount: 0, note: "", findings: [] },
      fullMigration2035: { verdict: "on-track", count: 0, findings: [] },
    });
    const csv = toCsv(FINDING_EXPORT_COLUMNS, findingExportRows(report), findingExportMeta(report, TAKEN_AT, ENV));
    const dataLine = csv.split("\n").filter((l) => l.startsWith("RSA-2048") || l.includes("RSA-2048"))[0];
    expect(dataLine).toContain(",,");
    expect(dataLine).not.toContain(",-,");
  });
});

describe("findingExportMeta", () => {
  it("says the milestone counts are not the row count", () => {
    const c = caveatsOf(findingExportMeta(ncscReport(), TAKEN_AT, ENV));
    expect(c.toLowerCase()).toContain("row count");
  });

  it("names, as a caveat, each milestone whose list is missing under a real count", () => {
    // 2031 counts 1 and carries no list, 2035 counts 2 and carries no list.
    const c = caveatsOf(findingExportMeta(ncscReport(), TAKEN_AT, ENV));
    expect(c).toContain("2031");
    expect(c).toContain("2035");
    expect(c).toMatch(/PARTIAL/);
  });

  it("does not cry partial when every milestone with a count carried its list", () => {
    const report = ncscReport({
      highestPriority2031: { verdict: "on-track", criteria: "", count: 0, migratedCount: 0, remainingCount: 0, note: "", findings: [] },
      fullMigration2035: { verdict: "on-track", count: 0, findings: [] },
    });
    expect(caveatsOf(findingExportMeta(report, TAKEN_AT, ENV))).not.toMatch(/PARTIAL/);
  });

  it("carries the report's own generation time, not only the download time", () => {
    const meta = findingExportMeta(ncscReport(), TAKEN_AT, ENV);
    expect(meta.takenAt).toBe(TAKEN_AT);
    expect(meta.windows.join("\n")).toContain("2026-07-17T10:00:00Z");
  });
});

describe("cbomExportRows", () => {
  const cbom = {
    bomFormat: "CycloneDX",
    specVersion: "1.6",
    components: [
      {
        name: "crypto/rsa",
        type: "cryptographic-asset",
        version: "1.22",
        cryptoProperties: { assetType: "algorithm", algorithmProperties: { primitive: "pke", parameterSetIdentifier: "2048" } },
      },
      { name: "crypto/ed25519" },
      "not-a-component",
    ],
  };

  it("reads the components the table reads, through the same function", () => {
    expect(cbomExportRows(cbom)).toHaveLength(cbomComponents(cbom).length);
  });

  it("carries the six fields this console understands", () => {
    const first = cbomExportRows(cbom)[0];
    expect(first).toEqual({
      name: "crypto/rsa",
      type: "cryptographic-asset",
      version: "1.22",
      asset_type: "algorithm",
      primitive: "pke",
      parameter_set: "2048",
    });
    expect(CBOM_EXPORT_COLUMNS).toHaveLength(6);
  });

  it("leaves a field the component did not carry unrecorded, not '-'", () => {
    const second = cbomExportRows(cbom)[1];
    expect(second.version).toBeNull();
    expect(second.asset_type).toBeNull();
    expect(second.primitive).toBeNull();
  });

  it("yields nothing at all from a document with no components array", () => {
    expect(cbomExportRows({ bomFormat: "CycloneDX" })).toEqual([]);
    expect(cbomExportRows(null)).toEqual([]);
  });
});

describe("cbomExportMeta", () => {
  it("says the file is the fields this console understands, not the whole CBOM", () => {
    const c = caveatsOf(cbomExportMeta({ components: [] }, "/repo", TAKEN_AT, ENV)).toLowerCase();
    expect(c).toContain("cyclonedx");
    expect(c).toMatch(/not the whole|only the fields|this console understands/);
  });

  it("says the document crossed the backend untyped", () => {
    expect(caveatsOf(cbomExportMeta({ components: [] }, "/repo", TAKEN_AT, ENV)).toLowerCase()).toContain("untyped");
  });

  it("warns when no components array was found, rather than reading as an empty inventory", () => {
    const found = caveatsOf(cbomExportMeta({ components: [{ name: "x" }] }, "/repo", TAKEN_AT, ENV));
    const missing = caveatsOf(cbomExportMeta({ bomFormat: "CycloneDX" }, "/repo", TAKEN_AT, ENV));
    expect(missing).toMatch(/PARTIAL|no top-level/i);
    expect(found).not.toMatch(/no top-level/i);
  });

  it("names the scanned target in a window line", () => {
    expect(cbomExportMeta({ components: [] }, "/srv/app", TAKEN_AT, ENV).windows.join("\n")).toContain("/srv/app");
  });
});

describe("qualityRunExportRows", () => {
  it("exports one row per run with the columns the table shows", () => {
    const rows = qualityRunExportRows([runSummary(), runSummary({ run: { id: "run-2", model: "sonnet", started_at: "x", finished_at: null } })]);
    expect(rows.map((r) => r.run_id)).toEqual(["run-1", "run-2"]);
    expect(QUALITY_RUN_EXPORT_COLUMNS.map((c) => c.key)).toContain("mean_score");
  });

  it("keeps an unfinished run unrecorded rather than stamping a finish time", () => {
    const rows = qualityRunExportRows([runSummary({ run: { id: "r", model: "m", started_at: "s", finished_at: null } })]);
    expect(rows[0].finished_at).toBeNull();
  });

  it("keeps a run with no scores at mean_score unrecorded, never 0", () => {
    // verdryx's AVG(value) is NULL for a run with no scores; a 0 here would
    // read as a run that scored zero.
    const rows = qualityRunExportRows([runSummary({ case_count: 0, mean_score: null, total_tokens: 0, total_cost_usd: 0 })]);
    expect(rows[0].mean_score).toBeNull();
    expect(rows[0].total_cost_usd).toBe(0);
  });
});

describe("qualityRunExportMeta", () => {
  it("says an empty mean_score is not a score of zero", () => {
    const c = caveatsOf(qualityRunExportMeta([runSummary()], "/db/verdryx.db", TAKEN_AT, ENV)).toLowerCase();
    expect(c).toContain("mean_score");
    expect(c).toMatch(/not a score of 0|not a score of zero/);
  });

  it("says a zero cost over no scores is an empty sum", () => {
    // COALESCE(SUM(cost_usd), 0.0) in crates/connectors/src/verdryx.rs.
    const c = caveatsOf(qualityRunExportMeta([runSummary()], "/db/verdryx.db", TAKEN_AT, ENV)).toLowerCase();
    expect(c).toContain("empty sum");
  });

  it("names the verdryx.db the rows came from", () => {
    expect(qualityRunExportMeta([runSummary()], "/db/verdryx.db", TAKEN_AT, ENV).windows.join("\n")).toContain("/db/verdryx.db");
  });
});

describe("baselineExportRows", () => {
  const baseline: VerdryxBaseline = {
    id: "b-1",
    eval_run_id: "run-1",
    mean_score: 0.77,
    created_at: "2026-08-02T10:00:00Z",
    label: "release-gate",
  };

  it("resolves the source run's model when the run is loaded", () => {
    expect(baselineExportRows([baseline], [runSummary()])[0].source_run_model).toBe("opus");
  });

  it("leaves the source model unrecorded when the run is not in the loaded list", () => {
    const row = baselineExportRows([baseline], [runSummary({ run: { id: "other", model: "sonnet", started_at: "s", finished_at: null } })])[0];
    expect(row.source_run_model).toBeNull();
    expect(row.eval_run_id).toBe("run-1");
  });

  it("leaves an unlabeled baseline's label unrecorded, not '(unlabeled)'", () => {
    expect(baselineExportRows([{ ...baseline, label: "  " }], [runSummary()])[0].label).toBeNull();
  });

  it("has a column for every field it fills", () => {
    expect(BASELINE_EXPORT_COLUMNS.map((c) => c.key).sort()).toEqual(Object.keys(baselineExportRows([baseline], null)[0]).sort());
  });
});

describe("baselineExportMeta", () => {
  it("says an empty source model is a join that missed, not a baseline with no run", () => {
    const c = caveatsOf(baselineExportMeta([], null, "/db/verdryx.db", TAKEN_AT, ENV)).toLowerCase();
    expect(c).toContain("source_run_model");
    expect(c).toMatch(/join/);
  });

  it("says the join had no runs at all when the runs list was never loaded", () => {
    const withRuns = caveatsOf(baselineExportMeta([], [runSummary()], "/db/verdryx.db", TAKEN_AT, ENV));
    const without = caveatsOf(baselineExportMeta([], null, "/db/verdryx.db", TAKEN_AT, ENV));
    expect(without).toMatch(/PARTIAL/);
    expect(withRuns).not.toMatch(/PARTIAL/);
  });
});

describe("routineHistoryExportRows", () => {
  it("exports the record fields the panel drops as well as the ones it shows", () => {
    const row = routineHistoryExportRows([routineRun()])[0];
    expect(row.artifact).toBe("out/qryx-trend-2026-08-20.json");
    expect(row.schema).toBe("stackup.routine-run/v1");
    expect(ROUTINE_HISTORY_EXPORT_COLUMNS.map((c) => c.key)).toContain("artifact");
  });

  it("keeps an unrecorded reason unrecorded rather than blank-looking", () => {
    expect(routineHistoryExportRows([routineRun()])[0].reason).toBeNull();
  });

  it("keeps a status it does not recognise, rather than folding it to unknown", () => {
    expect(routineHistoryExportRows([routineRun({ status: "quarantined" })])[0].status).toBe("quarantined");
  });
});

describe("routineHistoryExportMeta", () => {
  function history(records: RoutineRunDto[], skipped = 0): RoutinesHistoryDto {
    return { records, skipped_lines: skipped, routines_dir: "/stack-up/routines", history_file_exists: true };
  }

  it("says this is one routine, not all five", () => {
    const c = caveatsOf(routineHistoryExportMeta("qryx-trend", history([routineRun()]), TAKEN_AT, ENV));
    expect(c).toContain("qryx-trend");
    expect(c.toLowerCase()).toMatch(/one routine|this routine/);
  });

  it("declares the server's 200-record default cap", () => {
    const c = caveatsOf(routineHistoryExportMeta("qryx-trend", history([routineRun()]), TAKEN_AT, ENV));
    expect(c).toContain("200");
  });

  it("calls the file PARTIAL once the cap is actually reached", () => {
    const full = history(Array.from({ length: 200 }, () => routineRun()));
    const under = history(Array.from({ length: 199 }, () => routineRun()));
    expect(caveatsOf(routineHistoryExportMeta("qryx-trend", full, TAKEN_AT, ENV))).toMatch(/PARTIAL/);
    expect(caveatsOf(routineHistoryExportMeta("qryx-trend", under, TAKEN_AT, ENV))).not.toMatch(/PARTIAL/);
  });

  it("carries the lines history.ndjson could not parse into the file's own caveats", () => {
    const c = caveatsOf(routineHistoryExportMeta("qryx-trend", history([routineRun()], 3), TAKEN_AT, ENV));
    expect(c).toMatch(/3 line/);
  });

  it("says the history file does not exist rather than exporting silence", () => {
    const none: RoutinesHistoryDto = { records: [], skipped_lines: 0, routines_dir: "/stack-up/routines", history_file_exists: false };
    expect(caveatsOf(routineHistoryExportMeta("qryx-trend", none, TAKEN_AT, ENV)).toLowerCase()).toContain("does not exist");
  });
});

// ============================================================================
// 5. Every provenance block is complete enough to be a document
// ============================================================================

describe("every export's provenance block", () => {
  const metas = [
    ["findings", findingExportMeta(ncscReport(), TAKEN_AT, ENV)],
    ["cbom", cbomExportMeta({ components: [] }, "/repo", TAKEN_AT, ENV)],
    ["quality runs", qualityRunExportMeta([runSummary()], "/db/verdryx.db", TAKEN_AT, ENV)],
    ["baselines", baselineExportMeta([], [runSummary()], "/db/verdryx.db", TAKEN_AT, ENV)],
    [
      "routine history",
      routineHistoryExportMeta(
        "qryx-trend",
        { records: [routineRun()], skipped_lines: 0, routines_dir: "/r", history_file_exists: true },
        TAKEN_AT,
        ENV,
      ),
    ],
  ] as const;

  it.each(metas)("%s names a subject, an environment, a window and at least one caveat", (_name, meta) => {
    expect(meta.subject.length).toBeGreaterThan(0);
    expect(meta.environment).toBe(ENV);
    expect(meta.takenAt).toBe(TAKEN_AT);
    expect(meta.windows.length).toBeGreaterThan(0);
    expect((meta.caveats ?? []).length).toBeGreaterThan(0);
  });

  it.each(metas)("%s writes its provenance into the csv itself", (_name, meta) => {
    const csv = toCsv([{ key: "a" as never, header: "a" }], [], meta);
    expect(csv).toContain(`# subject: ${meta.subject}`);
    expect(csv).toContain(`# taken_at: ${TAKEN_AT}`);
    expect(csv).toContain("# caveat: ");
  });
});
