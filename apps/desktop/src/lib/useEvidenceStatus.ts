import { useEffect, useState } from "react";
import { fetchEvidenceStatus } from "./evidence";
import type { EvidenceStatus } from "../evidenceTypes";

/** Poll cadence while the backend is still resolving - mirrors
 * `lib/useDrillsStatus.ts`'s identical constant. In practice
 * `evidence::state::bootstrap` never actually awaits anything (three cheap
 * filesystem checks - see its own doc comment), so this settles on the very
 * first tick; kept for consistency with every other panel's status hook. */
const POLL_MS = 3_000;

/** Give up re-polling after this many attempts (~60s) - mirrors
 * `lib/useDrillsStatus.ts`'s identical rationale. */
const MAX_POLLS = 20;

/**
 * Shared connection-state hook for the Evidence view: fetches
 * `evidence_status` once, and re-polls every [`POLL_MS`] while the backend is
 * still bootstrapping. Structurally identical to `lib/useDrillsStatus.ts`'s
 * `useDrillsStatus`, duplicated rather than generalized since `EvidenceStatus`
 * is its own independent Rust-side type.
 */
export function useEvidenceStatus(): EvidenceStatus | null {
  const [status, setStatus] = useState<EvidenceStatus | null>(null);

  useEffect(() => {
    let cancelled = false;
    let pollCount = 0;
    let timer: ReturnType<typeof setTimeout> | undefined;

    const tick = async () => {
      const next = await fetchEvidenceStatus();
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
