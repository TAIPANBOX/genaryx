import { hasBackend, invokeBackend } from "./transport";
import type { CopilotAnswer, CopilotStatus } from "../copilotTypes";

/** The honest "nothing to talk to" status this module returns outside Tauri
 * (a plain `vite build`/browser preview) - mirrors every other panel's
 * `NO_ENVIRONMENT_ERROR`-style guard, never a fabricated enabled state. */
const NO_TAURI_STATUS: CopilotStatus = {
  enabled: false,
  provider: null,
  model: null,
  endpoint: null,
  local: null,
  disabled_reason: "No Tauri runtime to talk to.",
};

/** Whole-panel status for the residency banner. Never throws: outside Tauri
 * (or on any IPC failure) it resolves to a renderable disabled status
 * instead - mirrors `lib/identity.ts`'s `fetchIdentityStatus`. */
export async function fetchCopilotStatus(): Promise<CopilotStatus> {
  if (!hasBackend()) return NO_TAURI_STATUS;
  try {
    return await invokeBackend<CopilotStatus>("copilot_status");
  } catch (err) {
    return {
      enabled: false,
      provider: null,
      model: null,
      endpoint: null,
      local: null,
      disabled_reason: err instanceof Error ? err.message : String(err),
    };
  }
}

/** One question/answer round trip through Felyx. Unlike every other panel's
 * mutating/reading commands, `copilot_ask` rejects with a plain `String` on
 * the Rust side (`src-tauri/src/copilot/commands.rs`'s doc comment), not a
 * structured error DTO - `Answer` already derives `Serialize`, so a success
 * crosses the Tauri IPC boundary as-is. Callers should render a rejection as
 * an assistant note (e.g. "no copilot provider is configured..." when
 * nothing is set up), never as a crash - see `describeCopilotError` and
 * `CopilotView.tsx`. */
export async function askCopilot(question: string): Promise<CopilotAnswer> {
  if (!hasBackend()) throw new Error("No Tauri runtime to talk to.");
  return await invokeBackend<CopilotAnswer>("copilot_ask", { question });
}

/** The C1 "Explain with Felyx" cross-plane root-cause flow
 * (`CopilotService::explain_incident`, docs/PHASE6-C1.md): the same
 * one-round-trip shape as [`askCopilot`], just a different Tauri command
 * seeded with a fixed, incident-focused prompt built entirely on the Rust
 * side (`src-tauri/src/copilot/commands.rs::copilot_explain`) rather than
 * composed here. `incident_id` is snake_case on the wire (the Rust command
 * pins `rename_all = "snake_case"`, matching this app's IPC convention -
 * see `money.ts`'s identical note). Same rejection contract as
 * `askCopilot`: throws outside Tauri and lets any IPC rejection propagate as
 * a plain string/Error - callers render that rejection as an assistant note
 * via `describeCopilotError`, same as `askCopilot`. */
export async function explainIncident(incidentId: string): Promise<CopilotAnswer> {
  if (!hasBackend()) throw new Error("No Tauri runtime to talk to.");
  return await invokeBackend<CopilotAnswer>("copilot_explain", { incident_id: incidentId });
}

/** Human-readable text for whatever `askCopilot` rejected with. Tauri passes
 * a `Result::Err(String)` command's rejection through as that bare string
 * already, so this is normally an identity function; the fallback branches
 * only matter for a transport-level IPC failure. */
export function describeCopilotError(err: unknown): string {
  if (typeof err === "string") return err;
  if (err instanceof Error) return err.message;
  return String(err);
}

/** What `copilot_log_proposal_approved` reports back - mirrors
 * `evidence::commands::EvidenceBuildDto`'s `journaled`/`journal_error`
 * pairing (`src-tauri/src/evidence/commands.rs`): whether the audit link
 * itself got journaled is reported honestly, never thrown, so a journaling
 * hiccup can never make an already-successful, already-signed mutation look
 * like it failed. */
export interface ProposalApprovedOutcome {
  journaled: boolean;
  journal_error: string | null;
}

/** C2's audit link (docs/PHASE6-C2.md "Audit metadata"): called by
 * `CopilotView.tsx`'s `handleApproveProposal` right AFTER a proposal card's
 * Approve action has already completed the real signed mutation through its
 * own existing command (`killRun`/`setBudget`/`decideApproval`/`rescan`) -
 * never before, and never in place of it. Journals one
 * `console.copilot_proposal_approved` `CommandRecord` on the Rust side
 * (`src-tauri/src/copilot/commands.rs::copilot_log_proposal_approved`), so
 * the audit trail reads "human X approved copilot proposal Y", never
 * "copilot did Z" - see `crates/copilot/src/action.rs`'s doc comment: the
 * copilot crate holds no signer and never calls this (or any mutation) path
 * itself.
 *
 * Never throws: outside Tauri, or on any IPC failure, this resolves to an
 * honest `{journaled: false, journal_error: ...}` rather than rejecting -
 * the underlying mutation already succeeded by the time this is called, so a
 * transport hiccup here must never read as the approval itself having
 * failed. Mirrors `fetchCopilotStatus`'s identical never-throws contract. */
export async function logProposalApproved(
  kind: string,
  target: string,
  params: unknown,
): Promise<ProposalApprovedOutcome> {
  if (!hasBackend()) {
    return { journaled: false, journal_error: "No Tauri runtime to talk to." };
  }
  try {
    return await invokeBackend<ProposalApprovedOutcome>("copilot_log_proposal_approved", { kind, target, params });
  } catch (err) {
    return {
      journaled: false,
      journal_error: err instanceof Error ? err.message : String(err),
    };
  }
}
