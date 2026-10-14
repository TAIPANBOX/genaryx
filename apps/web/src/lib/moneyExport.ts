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

/** The model of the run's LAST call, as the Cloud aggregated it, or an honest
 * absence. Not "the model that priced the run": a run can switch models, and
 * the Cloud keeps one id, the last it ingested (forge lab, 2026-10-14: an
 * Opus-then-Haiku run reads Haiku). `lib/runStatus.ts`'s `runModelSummary`
 * is what a view prints.
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

/** The human at the root of this run's delegation chain, or an honest
 * absence.
 *
 * `RunAgg.owner` is `""` on a Cloud older than the field (`serde(default)`),
 * which is why this reads "not reported" and not "no owner resolved": unlike
 * `unit` (where an empty string is the Cloud's own answer, "nothing
 * resolved"), an empty owner does not distinguish "this Cloud resolved none"
 * from "this Cloud was never asked" - the money plane's separate `/v1/owners`
 * view is where "resolved and nobody" lives, as the literal `"unassigned"`
 * bucket. */
/** True when a run shows no budget because the Cloud's budget map could not
 * be read, not because the run has none (invariant 17). Every view that prints
 * a run's budget asks this before it writes "no cap". */
export function budgetUnknown(run: Run): boolean {
  return run.budget_usd === null && !run.budgets_read;
}

/** The hover text beside an unknown budget, said once so every view agrees. */
export const BUDGET_UNKNOWN_TITLE =
  "TokenFuse Cloud did not answer GET /v1/budgets, so this run's budget could not be read. Only budgets near or over their alert threshold are known.";

export function runOwnerLabel(run: Run): string {
  return run.owner && run.owner.length > 0 ? run.owner : "not reported";
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
  /** `null` when the Cloud sent nothing (older Cloud, or never asked to
   * attribute one) - see [`runOwnerLabel`]'s doc comment for why this reads
   * differently from `unit`'s own empty-string case. */
  owner: string | null;
  model: string | null;
  spent_usd: number;
  budget_usd: number | null;
  budgets_read: boolean;
  calls: number | null;
  cache_hits: number | null;
  steps: number | null;
  last_seen: string;
  killed: boolean;
  // How the gateway decided the run (TokenFuse Cloud v1.2.0 and v1.8.0).
  // Appended after the columns this file already had, never reordered, so a
  // reader of an older export keeps its column positions. `null` (an empty
  // cell) wherever the Cloud did not report the field.
  site: string | null;
  blocked: number | null;
  last_decision: string | null;
  last_decision_at: string | null;
  fallback_calls: number | null;
  basis_unreported_calls: number | null;
  identity_warned: number | null;
  last_identity_reason: string | null;
  /** Every model the run used, `model:calls:spent_usd` joined by `;`, or
   * `null` when the Cloud does not report them (then only `model`, the last
   * call's, is known). */
  models: string | null;
  model_switches: number | null;
}

export const RUNS_EXPORT_COLUMNS: { key: keyof RunExportRow & string; header: string }[] = [
  { key: "run_id", header: "run_id" },
  { key: "agent_id", header: "agent_id" },
  { key: "unit", header: "unit" },
  { key: "owner", header: "owner" },
  { key: "model", header: "model" },
  { key: "spent_usd", header: "spent_usd" },
  { key: "budget_usd", header: "budget_usd" },
  { key: "budgets_read", header: "budgets_read" },
  { key: "calls", header: "calls" },
  { key: "cache_hits", header: "cache_hits" },
  { key: "steps", header: "steps" },
  { key: "last_seen", header: "last_seen" },
  { key: "killed", header: "killed" },
  { key: "site", header: "site" },
  { key: "blocked", header: "blocked" },
  { key: "last_decision", header: "last_decision" },
  { key: "last_decision_at", header: "last_decision_at" },
  { key: "fallback_calls", header: "fallback_calls" },
  { key: "basis_unreported_calls", header: "basis_unreported_calls" },
  { key: "identity_warned", header: "identity_warned" },
  { key: "last_identity_reason", header: "last_identity_reason" },
  { key: "models", header: "models" },
  { key: "model_switches", header: "model_switches" },
];

function orNull(value: string | null | undefined): string | null {
  return value !== null && value !== undefined && value.length > 0 ? value : null;
}

function numberOrNull(value: number | null | undefined): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

/** A string the Cloud reported, kept verbatim even when empty (`""` is an
 * answer for `site` and `last_decision`), or `null` when it reported none. */
function reportedOrNull(value: string | null | undefined): string | null {
  return typeof value === "string" ? value : null;
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
    owner: orNull(r.owner),
    model: orNull(r.model),
    spent_usd: r.spent_usd,
    budget_usd: numberOrNull(r.budget_usd),
    budgets_read: r.budgets_read,
    calls: numberOrNull(r.calls),
    cache_hits: numberOrNull(r.cache_hits),
    steps: numberOrNull(r.steps),
    last_seen: r.last_seen,
    killed: r.killed,
    site: reportedOrNull(r.site),
    blocked: numberOrNull(r.blocked),
    last_decision: reportedOrNull(r.last_decision),
    last_decision_at: reportedOrNull(r.last_decision_at),
    fallback_calls: numberOrNull(r.fallback_calls),
    basis_unreported_calls: numberOrNull(r.basis_unreported_calls),
    identity_warned: numberOrNull(r.identity_warned),
    last_identity_reason: reportedOrNull(r.last_identity_reason),
    models: Array.isArray(r.models) ? r.models.map((m) => `${m.model}:${m.calls}:${m.spent_usd}`).join(";") : null,
    model_switches: numberOrNull(r.model_switches),
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
      "budget_usd is the run's budget on the Cloud (GET /v1/budgets). An empty budget_usd on a row whose budgets_read is true means the Cloud holds no budget for that run. On a row whose budgets_read is false the Cloud's budget map could not be read for this file, only the budgets GET /v1/alerts lists are known, and an empty budget_usd means unknown, never that the run has none.",
      "An empty unit means the Cloud's identity map resolved none for that run. The run was still charged whatever the Cloud charged it.",
      "An empty owner means this Cloud sent no owner for that run at all (an older Cloud, or one never asked to attribute one). It is not the same claim as an empty unit: this console cannot tell 'resolved and nobody' from 'never asked' through this field.",
      "An empty agent_id is what the money plane carried for that run, not a value this console dropped.",
      "model is the model of the run's LAST call as the Cloud aggregated it, not the model of the run: a run can switch models. An empty model means the run's aggregate carried none. models lists every model the run used (model:calls:spent_usd, joined by ;) only when the Cloud reports it; an empty models means it does not, and whether the run used other models is then unknown. model_switches counts the changes, when reported.",
      "An empty cache_hits means the field never arrived. A cache_hits of 0 means the money plane counted no cache hit for that run. They are different statements and this file keeps them apart.",
      "An empty blocked, last_decision, fallback_calls, basis_unreported_calls, identity_warned, last_identity_reason or site means this Cloud does not report that field (site needs TokenFuse Cloud v1.2.0, the rest v1.8.0), never zero. A blocked of 0 is a count. last_decision is the latest call's decision: allow or cache_hit admitted it, anything else is the reason the gateway refused it.",
      "fallback_calls counts admitted calls the gateway charged at the fallback rate (15/75 USD per million tokens) because its price book had no row for the model id: their spend is a gateway estimate, not a provider bill. basis_unreported_calls counts calls from a gateway that named no price basis at all. identity_warned counts calls forwarded although the identity check would have refused them (warn mode); they are spent and filed under the agent id they claimed.",
      "killed is the money plane's own flag. A run an operator froze or stopped through this console shows its lifecycle on screen, and that state is this console's, not a column of GET /v1/runs.",
    ],
  };
}
