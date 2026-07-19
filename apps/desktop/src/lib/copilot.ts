import { invoke, isTauri } from "@tauri-apps/api/core";
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
  if (!isTauri()) return NO_TAURI_STATUS;
  try {
    return await invoke<CopilotStatus>("copilot_status");
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
  if (!isTauri()) throw new Error("No Tauri runtime to talk to.");
  return await invoke<CopilotAnswer>("copilot_ask", { question });
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
  if (!isTauri()) throw new Error("No Tauri runtime to talk to.");
  return await invoke<CopilotAnswer>("copilot_explain", { incident_id: incidentId });
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
