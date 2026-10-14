/**
 * How the gateway decided a run, in the words every view uses.
 *
 * TokenFuse Cloud v1.8.0 carries, on every `GET /v1/runs` row, the calls the
 * gateway refused (`blocked`), the decision of the run's latest call
 * (`last_decision`), the calls it charged at the fallback rate because its
 * price book had no row for the model (`fallback_calls`), the calls whose
 * gateway named no price basis at all (`basis_unreported_calls`), the calls
 * forwarded although the identity check would have refused them
 * (`identity_warned`, warn mode), and since v1.2.0 the site that pushed the
 * run. The console dropped every one of them until 2026-10-14, so a run whose
 * every call was refused read "live".
 *
 * A field the Cloud did not report is `null` or absent, and these helpers
 * print nothing rather than a zero for it: a zero is a count somebody made.
 * The runs board, Incident 360 and the export read the fields only through
 * here, so the three cannot disagree.
 */

import type { Run } from "../moneyTypes";

/** The two decisions that admitted a call. Anything else is a refusal. */
const ADMITTED = new Set(["allow", "cache_hit"]);

/** A refusal reason -> the short word on the status pill. The raw wire string
 * stays in the hover text. A reason with no word here shows as itself, which
 * is the Cloud's own wording rather than a guess. The same table TokenFuse's
 * own dashboard keeps. */
const REFUSAL_WORDS: Record<string, string> = {
  budget_exceeded: "budget",
  policy_violation: "policy",
  loop_detected: "loop",
  killed: "killed",
  wasm_policy: "policy",
  taint_blocked: "taint",
  dlp_blocked: "dlp",
  unit_budget_exceeded: "unit cap",
  identity_mismatch: "identity",
};

/** A string from outside the process shown in a cell: bounded, so a Cloud
 * that sends a paragraph cannot push the row apart. React escapes it. */
function bounded(s: string, max = 48): string {
  return s.length > max ? `${s.slice(0, max - 1)}…` : s;
}

function count(n: number | null | undefined): number | null {
  return typeof n === "number" && Number.isFinite(n) && n >= 0 ? n : null;
}

function text(s: string | null | undefined): string | null {
  return typeof s === "string" ? s : null;
}

/** Whether this Cloud reports refusals at all (v1.8.0 or later). */
export function decisionsReported(run: Run): boolean {
  return count(run.blocked) !== null;
}

/** The latest call's refusal, or `null` when it was admitted, when no decision
 * was seen yet, or when this Cloud does not report decisions. */
export function latestRefusal(run: Run): { decision: string; word: string; title: string } | null {
  const d = text(run.last_decision);
  if (!d || ADMITTED.has(d)) return null;
  const word = bounded(REFUSAL_WORDS[d] ?? d, 24);
  const reason = text(run.last_identity_reason);
  const when = text(run.last_decision_at);
  const title =
    `The latest call on this run was refused: ${bounded(d, 64)}` +
    (d === "identity_mismatch" && reason ? ` (${bounded(reason, 64)})` : "") +
    (when ? `, at ${when}` : "") +
    ". The gateway refused it before any provider was called.";
  return { decision: d, word, title };
}

/** "N refused" under the call count, or `null` for zero and for a Cloud that
 * does not report refusals (the status pill's hover says which). */
export function refusedCountLabel(run: Run): string | null {
  const n = count(run.blocked);
  return n !== null && n > 0 ? `${n.toLocaleString("en-US")} refused` : null;
}

/** The status pill's hover when nothing more specific applies. */
export function statusTitle(run: Run): string | undefined {
  if (decisionsReported(run)) return undefined;
  return "This Cloud does not report refused calls (it is older than TokenFuse v1.8.0), so live means only that the run was not killed and is under its cap.";
}

/** One small pill beside the status: a fact about how calls were charged or
 * admitted that an operator would otherwise not see. */
export interface RunSignal {
  key: "fallback" | "basis_unreported" | "identity_warn";
  label: string;
  title: string;
}

/** The signal pills for one run, in a fixed order. A field the Cloud did not
 * report, or reported as zero, yields no pill. */
export function runSignals(run: Run): RunSignal[] {
  const out: RunSignal[] = [];
  const fb = count(run.fallback_calls);
  if (fb !== null && fb > 0) {
    out.push({
      key: "fallback",
      label: `fallback ×${fb}`,
      title: `${fb} call(s) charged at the fallback rate: the gateway's price book has no row for this model id, so it charged the conservative 15/75 USD per million tokens. A gateway estimate, not a provider bill.`,
    });
  }
  const nb = count(run.basis_unreported_calls);
  if (nb !== null && nb > 0) {
    out.push({
      key: "basis_unreported",
      label: `no basis ×${nb}`,
      title: `${nb} call(s) from a gateway that named no price basis (one older than TokenFuse v1.8.0), so whether they were charged at the fallback rate cannot be told.`,
    });
  }
  const iw = count(run.identity_warned);
  if (iw !== null && iw > 0) {
    const reason = text(run.last_identity_reason);
    out.push({
      key: "identity_warn",
      label: `id warn ×${iw}`,
      title:
        `${iw} call(s) forwarded although the identity check would have refused them (identity warn mode)` +
        (reason ? `: ${bounded(reason, 64)}` : "") +
        ". They are spent and filed under the agent id they claimed.",
    });
  }
  return out;
}

/** How much a run asks for an operator's attention, for the board's "status"
 * sort: killed 3, latest call refused 2, any signal pill 1, otherwise 0. A
 * refused run used to sort with the live ones, so "sort by status" could not
 * find it. */
export function runAttentionRank(run: Run): number {
  if (run.killed) return 3;
  if (latestRefusal(run)) return 2;
  return runSignals(run).length > 0 ? 1 : 0;
}

/** The site that pushed the run, or what the Cloud's answer means. */
export function runSiteLabel(run: Run): string {
  const s = text(run.site);
  if (s === null) return "site not reported";
  return s.length > 0 ? `site ${bounded(s)}` : "no site named";
}

/** One sentence for Incident 360's "What the whole run cost": how the gateway
 * decided the run, or that this Cloud does not say. */
export function runDecisionSentence(run: Run): string {
  if (!decisionsReported(run)) {
    return "This Cloud does not report refusals, price basis or identity warnings for a run (it is older than TokenFuse v1.8.0).";
  }
  const parts: string[] = [];
  const refusal = latestRefusal(run);
  const d = text(run.last_decision);
  if (refusal) parts.push(`latest call refused: ${bounded(refusal.decision, 64)}`);
  else if (d) parts.push(`latest call admitted (${bounded(d, 24)})`);
  else parts.push("no decision recorded yet");
  const refused = count(run.blocked) ?? 0;
  parts.push(refused === 1 ? "1 call refused" : `${refused.toLocaleString("en-US")} calls refused`);
  for (const s of runSignals(run)) {
    if (s.key === "fallback") parts.push(`${count(run.fallback_calls)} charged at the fallback rate`);
    if (s.key === "basis_unreported") parts.push(`${count(run.basis_unreported_calls)} with no price basis named`);
    if (s.key === "identity_warn") {
      const reason = text(run.last_identity_reason);
      parts.push(
        `${count(run.identity_warned)} forwarded under identity warn` + (reason ? ` (${bounded(reason, 64)})` : ""),
      );
    }
  }
  return parts.join(" · ");
}
