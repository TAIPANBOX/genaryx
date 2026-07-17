import { invoke, isTauri } from "@tauri-apps/api/core";
import type { UiEvent } from "../types";

/**
 * One run's events, oldest-first (chronological - the order Run Replay plays
 * forward through). Never throws: outside Tauri, or on any IPC failure, this
 * resolves to an empty list rather than an error - matching `run_events`'s
 * own "never an Err the UI traps on" contract on the Rust side
 * (`src-tauri/src/replay.rs`), the same convention `lib/graph.ts`'s
 * `fetchAgentEvents` already follows for its sibling command. There is no
 * plausible mock replay timeline to fall back to (same reasoning
 * `lib/graph.ts`'s own doc comment gives for the delegation graph), so
 * "empty" is the one honest fallback both inside and outside Tauri.
 */
export async function fetchRunEvents(runId: string, limit: number): Promise<UiEvent[]> {
  if (!isTauri()) return [];
  try {
    return await invoke<UiEvent[]>("run_events", { run_id: runId, limit });
  } catch (err) {
    // eslint-disable-next-line no-console
    console.error(`run_events invoke failed for ${runId}, rendering no events:`, err);
    return [];
  }
}
