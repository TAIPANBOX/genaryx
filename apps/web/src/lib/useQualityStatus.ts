import { useEffect, useState } from "react";
import { fetchQualityStatus } from "./quality";
import type { QualityStatus } from "../qualityTypes";

/** Poll cadence while the backend is still resolving (`state: "bootstrapping"`,
 * see `crates/api/src/quality/state.rs`'s non-blocking bootstrap). Mirrors
 * `lib/useIdentityStatus.ts`'s identical constant. */
const POLL_MS = 3_000;

/** Give up re-polling after this many attempts (~60s) - mirrors
 * `lib/useIdentityStatus.ts`'s identical rationale. */
const MAX_POLLS = 20;

/**
 * Shared connection-state hook for the Quality view: fetches
 * `quality_status` once, and re-polls every [`POLL_MS`] while the backend is
 * still bootstrapping. Settles (stops polling) the moment the state is
 * anything other than "bootstrapping". Structurally identical to
 * `lib/useIdentityStatus.ts`'s `useIdentityStatus`, duplicated rather than
 * generalized over every panel since `QualityStatus`/`IdentityStatus` are
 * independent Rust-side types that only happen to share this shape today.
 */
export function useQualityStatus(): QualityStatus | null {
  const [status, setStatus] = useState<QualityStatus | null>(null);

  useEffect(() => {
    let cancelled = false;
    let pollCount = 0;
    let timer: ReturnType<typeof setTimeout> | undefined;

    const tick = async () => {
      const next = await fetchQualityStatus();
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
