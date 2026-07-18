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

/** Human-readable text for whatever `askCopilot` rejected with. Tauri passes
 * a `Result::Err(String)` command's rejection through as that bare string
 * already, so this is normally an identity function; the fallback branches
 * only matter for a transport-level IPC failure. */
export function describeCopilotError(err: unknown): string {
  if (typeof err === "string") return err;
  if (err instanceof Error) return err.message;
  return String(err);
}
