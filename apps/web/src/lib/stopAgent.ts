import type { LiftResult, StopError, StopResult, StopStep } from "../delegationTypes";

/** One line of an outcome the operator reads. */
export interface ReportLine {
  tone: "ok" | "bad" | "warn";
  text: string;
}

/** A stop's or a lift's outcome, as the operator reads it. */
export interface StopReport {
  /** True only for a complete stop or lift. */
  done: boolean;
  headline: string;
  lines: ReportLine[];
}

/** One step as a line: its name, what it acted on, and what the service
 * answered. A step the chain did not take gets a second line saying so,
 * because an act that is not recorded is indistinguishable from one that did
 * not happen (invariant 3). */
function stepLines(steps: StopStep[]): ReportLine[] {
  const out: ReportLine[] = [];
  for (const st of steps) {
    const status = st.http_status > 0 ? ` (HTTP ${st.http_status})` : "";
    out.push({
      tone: st.ok ? "ok" : "bad",
      text: `${st.step} ${st.target}: ${st.verify_result}${status}`,
    });
    if (!st.journaled) {
      out.push({
        tone: "warn",
        text: `${st.step} was not recorded in the console's chain: ${st.journal_error ?? "no reason given"}`,
      });
    }
  }
  return out;
}

/** Whether `v` has the shape of an outcome the command sends: a subject and
 * a list of steps. The demo transport answers `null` for a command it has no
 * handler for, and a console pointed at the wrong box could answer anything;
 * neither may read as an action that happened. */
function hasSteps(v: unknown): v is { subject: string; steps: StopStep[]; warnings?: unknown } {
  if (!v || typeof v !== "object") return false;
  const o = v as { subject?: unknown; steps?: unknown };
  return typeof o.subject === "string" && Array.isArray(o.steps);
}

function warningLines(o: { warnings?: unknown }): ReportLine[] {
  return Array.isArray(o.warnings)
    ? o.warnings.map((w): ReportLine => ({ tone: "warn", text: String(w) }))
    : [];
}

const NO_OUTCOME: ReportLine = { tone: "bad", text: "the console answered without an outcome, so nothing is known to have happened" };

/** A refusal that carries no outcome: the command never ran a step. */
function refusalLine(error: StopError): ReportLine {
  switch (error.kind) {
    case "invalid_subject":
      return { tone: "bad", text: `not an agent id: ${error.subject}` };
    case "invalid_reason":
      return { tone: "bad", text: `a reason of 1 to 500 characters is required (got ${error.chars})` };
    case "role_required":
      return { tone: "bad", text: `role ${error.role} required` };
    case "failed":
      return { tone: "bad", text: error.detail };
    default:
      return { tone: "bad", text: "refused" };
  }
}

/** A stop's outcome. Done only when the command answered Ok: a stop with
 * any failed step comes back as `not_stopped`, and reads as NOT stopped with
 * the failed step in red, whatever the other steps did. */
export function stopReport(result: StopResult): StopReport {
  if (result.ok) {
    const o = result.outcome;
    // Done only for an outcome that says so itself: the backend answers Ok
    // only for a complete stop, and anything else is not one.
    if (!hasSteps(o) || o.stopped !== true) {
      return {
        done: false,
        headline: hasSteps(o) ? `NOT stopped: ${o.subject}` : "Agent NOT stopped",
        lines: hasSteps(o)
          ? [...stepLines(o.steps), ...warningLines(o), { tone: "bad", text: "the outcome does not say the agent was stopped" }]
          : [NO_OUTCOME],
      };
    }
    return {
      done: true,
      headline: `Stopped ${o.subject}: banned at the issuer, ${o.runs_killed} open run(s) killed`,
      lines: [...stepLines(o.steps), ...warningLines(o)],
    };
  }
  const e = result.error;
  if (e.kind === "not_stopped") {
    const o: unknown = e.outcome;
    if (!hasSteps(o)) return { done: false, headline: "Agent NOT stopped", lines: [NO_OUTCOME] };
    return {
      done: false,
      headline: `NOT stopped: ${o.subject}`,
      lines: [...stepLines(o.steps), ...warningLines(o)],
    };
  }
  return { done: false, headline: "Agent NOT stopped", lines: [refusalLine(e)] };
}

/** A lift's outcome, the same rule: done only when the command answered Ok
 * with an outcome that says it lifted. */
export function liftReport(result: LiftResult): StopReport {
  if (result.ok) {
    const o = result.outcome;
    if (!hasSteps(o) || o.lifted !== true) {
      return {
        done: false,
        headline: hasSteps(o) ? `Ban NOT lifted: ${o.subject}` : "Ban NOT lifted",
        lines: hasSteps(o)
          ? [...stepLines(o.steps), ...warningLines(o), { tone: "bad", text: "the outcome does not say the ban was lifted" }]
          : [NO_OUTCOME],
      };
    }
    return {
      done: true,
      headline: `Ban lifted for ${o.subject}`,
      lines: [...stepLines(o.steps), ...warningLines(o)],
    };
  }
  const e = result.error;
  if (e.kind === "not_lifted") {
    const o: unknown = e.outcome;
    if (!hasSteps(o)) return { done: false, headline: "Ban NOT lifted", lines: [NO_OUTCOME] };
    return {
      done: false,
      headline: `Ban NOT lifted: ${o.subject}`,
      lines: [...stepLines(o.steps), ...warningLines(o)],
    };
  }
  return { done: false, headline: "Ban NOT lifted", lines: [refusalLine(e)] };
}
