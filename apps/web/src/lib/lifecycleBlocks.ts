import { useEffect, useState } from "react";
import { useConsoleStateVersion } from "./consoleState";
import { hasBackend, invokeBackend } from "./transport";

/**
 * What a REAL box says it has blocked, straight from the console's own
 * lifecycle store (`lifecycle_blocks`, `crates/web/src/lifecycle.rs`).
 *
 * Why this exists at all: an agent's frozen state can be derived from the runs
 * a real box already serves (`money_runs` stamps `Run.lifecycle` on a blocked
 * agent's runs), but a UNIT's or a USER's cannot - the per-entity records that
 * carry `stopped` (`unit_record`/`user_record`) are preview-only commands with
 * no handler on a box, so a Stop would enforce correctly in wardryx and then
 * read back as "not stopped" the moment the card refetched, flipping the
 * button label back to Stop. This read is the box's answer to that: the same
 * durable set of blocks the enforcement wrote, served back so every surface
 * points the right way.
 *
 * Under the mock (the it-rat.com Live Demo) this command has no handler, so it
 * answers empty and the demo keeps deriving state from its own records, which
 * already carry `stopped`. Both paths therefore stay honest: neither invents a
 * block the other side does not have.
 */
export interface LifecycleBlocks {
  /** Frozen agent ids (`agent://org/team/name`). */
  agents: string[];
  /** Stopped business units, by team segment. */
  units: string[];
  /** Stopped users, by the handle the console pins them under. */
  users: string[];
}

const EMPTY: LifecycleBlocks = { agents: [], units: [], users: [] };

export async function fetchLifecycleBlocks(): Promise<LifecycleBlocks> {
  if (!hasBackend()) return EMPTY;
  try {
    const blocks = await invokeBackend<LifecycleBlocks | null>("lifecycle_blocks");
    if (!blocks) return EMPTY;
    return {
      agents: Array.isArray(blocks.agents) ? blocks.agents : [],
      units: Array.isArray(blocks.units) ? blocks.units : [],
      users: Array.isArray(blocks.users) ? blocks.users : [],
    };
  } catch {
    // No handler (mock, or a box that predates this command) is not an error:
    // an empty answer just means "this surface has nothing extra to add".
    return EMPTY;
  }
}

/** Re-reads whenever any lifecycle action lands anywhere in the app, so a Stop
 * issued in the watch dock points the unit card's button the right way too. */
export function useLifecycleBlocks(): LifecycleBlocks {
  const version = useConsoleStateVersion();
  const [blocks, setBlocks] = useState<LifecycleBlocks>(EMPTY);
  useEffect(() => {
    let cancelled = false;
    void fetchLifecycleBlocks().then((b) => {
      if (!cancelled) setBlocks(b);
    });
    return () => {
      cancelled = true;
    };
  }, [version]);
  return blocks;
}
