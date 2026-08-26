/**
 * What the Money tab shows about a run, and what leaves the console as a file.
 *
 * # WHY THESE LABELS ARE A MODULE AND NOT AN INLINE TERNARY
 *
 * Every value on a run row can be missing, and each one means something
 * different when it is. An absent `cache_hits` means the field never arrived;
 * a `cache_hits` of 0 means the money plane counted no cache hit for that run.
 * An empty `unit` is the Cloud's identity map answering "none", which is a real
 * answer and not a gap. Printing a 0, or a bare dash, for any of them turns a
 * question the console could not answer into a measurement somebody took.
 *
 * The labels live here rather than in `RunsBoard.tsx` so the same sentence is
 * used by the table and by the exported file, and so both can be tested without
 * a DOM.
 */

import type { ExportMeta } from "./download";
import type { Run, Savings } from "../moneyTypes";

/** What the console says where a field never reached it. Deliberately words
 * rather than a dash: this string appears beside real figures, with no legend
 * next to it to explain a symbol. */
export const NOT_RECORDED = "not recorded";

/** A count, or [`NOT_RECORDED`] when the box never sent one.
 *
 * The runtime guard is not defensive noise. `Run.cache_hits` is declared
 * non-optional in `moneyTypes.ts` because `RunAgg` in
 * `crates/connectors/src/cloud_rest.rs` declares it non-optional too, so a
 * Cloud that omits it fails deserialization outright rather than defaulting.
 * That is what the Rust side does today; it is not something this view can
 * check, and a TypeScript type is a claim about a JSON payload, not a
 * guarantee about one. If a payload ever arrives without the field, the cell
 * says so instead of rendering `NaN` or a confident 0. */
export function countLabel(n: number | null | undefined): string {
  return typeof n === "number" && Number.isFinite(n) ? n.toLocaleString("en-US") : NOT_RECORDED;
}

/** Cache hits for one run: a FinOps number that until now rendered nowhere in
 * this console, despite riding on every row of `GET /v1/runs`. */
export function cacheHitsLabel(run: Run): string {
  return countLabel(run.cache_hits);
}

/** The model the run's spend was aggregated under, or an honest absence.
 *
 * `RunAgg.model` is a plain string, so `""` is the Cloud recording none. */
export function runModelLabel(run: Run): string {
  return run.model && run.model.length > 0 ? run.model : NOT_RECORDED;
}

/** The business unit the Cloud's identity map RESOLVED for this run.
 *
 * `""` is the documented "nothing resolved" answer (`RunDto.unit`: "Empty and
 * absent are the same thing here and both mean do not claim a unit for this
 * run"), which is why this reads "no unit resolved" and not "not recorded":
 * the box answered, and its answer was none. Same wording `Incident360.tsx`
 * already uses for the same field. */
export function runUnitLabel(run: Run): string {
  return run.unit && run.unit.length > 0 ? run.unit : "no unit resolved";
}

/** The Governed savings caption on the Money tab.
 *
 * `Savings.budget_breaks` reaches this view on every refresh and rendered only
 * on Overview, so the Money tab showed less about savings than the summary
 * above it. "prevented + recovered" stays: it says what the composition below
 * is made of, and the break count says how often a budget actually tripped. */
export function governedSavingsCaption(savings: Savings): string {
  const breaks = savings.budget_breaks;
  if (typeof breaks !== "number" || !Number.isFinite(breaks)) {
    return `budget breaks ${NOT_RECORDED} · prevented + recovered`;
  }
  const word = breaks === 1 ? "budget break" : "budget breaks";
  return `${breaks.toLocaleString("en-US")} ${word} · prevented + recovered`;
}

// ---- The runs export -------------------------------------------------------

/** One exported run. Flat by design (`toCsv` addresses columns by key), and
 * every field is something `GET /v1/runs` actually carried.
 *
 * `null` rather than `""` or `0` wherever the value was absent: `download.ts`
 * writes `null` as an EMPTY cell precisely so it cannot be read as a figure. */
export interface RunExportRow {
  run_id: string;
  agent_id: string;
  unit: string | null;
  model: string | null;
  spent_usd: number;
  budget_usd: number | null;
  calls: number | null;
  cache_hits: number | null;
  steps: number | null;
  last_seen: string;
  killed: boolean;
}

export const RUNS_EXPORT_COLUMNS: { key: keyof RunExportRow & string; header: string }[] = [
  { key: "run_id", header: "run_id" },
  { key: "agent_id", header: "agent_id" },
  { key: "unit", header: "unit" },
  { key: "model", header: "model" },
  { key: "spent_usd", header: "spent_usd" },
  { key: "budget_usd", header: "budget_usd" },
  { key: "calls", header: "calls" },
  { key: "cache_hits", header: "cache_hits" },
  { key: "steps", header: "steps" },
  { key: "last_seen", header: "last_seen" },
  { key: "killed", header: "killed" },
];

function orNull(value: string | null | undefined): string | null {
  return value !== null && value !== undefined && value.length > 0 ? value : null;
}

function numberOrNull(value: number | null | undefined): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

/** The rows of the runs export.
 *
 * Takes the FULL list the money plane returned, not the slice the board shows:
 * a file that silently held the top eighteen of four hundred would be exactly
 * the "looks complete and is not" failure `download.ts` exists to prevent. The
 * meta block below says which one this is. */
export function runsExportRows(runs: Run[]): RunExportRow[] {
  return runs.map((r) => ({
    run_id: r.run_id,
    // Kept verbatim, empty string included: an empty agent_id is what the
    // money plane carried for that run, not a value this console lost.
    agent_id: r.agent_id,
    unit: orNull(r.unit),
    model: orNull(r.model),
    spent_usd: r.spent_usd,
    budget_usd: numberOrNull(r.budget_usd),
    calls: numberOrNull(r.calls),
    cache_hits: numberOrNull(r.cache_hits),
    steps: numberOrNull(r.steps),
    last_seen: r.last_seen,
    killed: r.killed,
  }));
}

/** Provenance for the runs export.
 *
 * `shown` and `total` are both stated because they differ: the board renders a
 * sorted top slice and this file carries everything the console received. A
 * reader days later cannot recover either number from the rows themselves. */
export function runsExportMeta(opts: {
  shown: number;
  total: number;
  environment: string;
  takenAt: string;
}): ExportMeta {
  return {
    subject: "Genaryx money runs",
    environment: opts.environment,
    takenAt: opts.takenAt,
    windows: [
      "runs, spend, calls and cache hits: the money plane's own window (TokenFuse Cloud), as GET /v1/runs aggregated it",
    ],
    caveats: [
      `This file carries every one of the ${opts.total.toLocaleString("en-US")} run(s) the money plane returned, not the ${opts.shown.toLocaleString("en-US")} the Runs table shows. The table is a sorted top slice; this is not.`,
      "This console does not page GET /v1/runs, so the file is exactly that endpoint's answer. Whether the Cloud itself caps or windows that array is the Cloud's own decision and is not visible from here.",
      "An empty budget_usd means this console could not learn a budget for the run, never that the run has none: a budget is knowable only once the Cloud's alert threshold has tripped for it, or somebody set one from this console in this session.",
      "An empty unit means the Cloud's identity map resolved none for that run. The run was still charged whatever the Cloud charged it.",
      "An empty agent_id is what the money plane carried for that run, not a value this console dropped.",
      "An empty model means the run's aggregate carried none.",
      "An empty cache_hits means the field never arrived. A cache_hits of 0 means the money plane counted no cache hit for that run. They are different statements and this file keeps them apart.",
      "killed is the money plane's own flag. A run an operator froze or stopped through this console shows its lifecycle on screen, and that state is this console's, not a column of GET /v1/runs.",
    ],
  };
}
