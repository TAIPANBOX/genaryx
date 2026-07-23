import { useEffect, useState } from "react";
import { fetchDrillsStatus } from "./drills";
import type { DrillsStatus } from "../drillsTypes";

/** Poll cadence while the backend is still resolving - mirrors
 * `lib/useCryptoStatus.ts`'s identical constant. In practice
 * `drills::state::bootstrap` never actually awaits anything (mirrors
 * `crypto::state::bootstrap`'s identical note), so this settles on the very
 * first tick; kept for consistency with every other panel's status hook. */
const POLL_MS = 3_000;

/** Give up re-polling after this many attempts (~60s) - mirrors
 * `lib/useCryptoStatus.ts`'s identical rationale. */
const MAX_POLLS = 20;

/**
 * Shared connection-state hook for the Drills view: fetches `drills_status`
 * once, and re-polls every [`POLL_MS`] while the backend is still
 * bootstrapping. Structurally identical to `lib/useCryptoStatus.ts`'s
 * `useCryptoStatus`, duplicated rather than generalized since `DrillsStatus`
 * is its own independent Rust-side type.
 */
export function useDrillsStatus(): DrillsStatus | null {
  const [status, setStatus] = useState<DrillsStatus | null>(null);

  useEffect(() => {
    let cancelled = false;
    let pollCount = 0;
    let timer: ReturnType<typeof setTimeout> | undefined;

    const tick = async () => {
      const next = await fetchDrillsStatus();
      if (cancelled) return;
      setStatus(next);
      pollCount += 1;
      if (next.state === "bootstrapping" && pollCount < MAX_POLLS) {
        timer = setTimeout(() => void tick(), POLL_MS);
      }
    };
    void tick();

    return () => {
      cancelled = true;
      if (timer !== undefined) clearTimeout(timer);
    };
  }, []);

  return status;
}
