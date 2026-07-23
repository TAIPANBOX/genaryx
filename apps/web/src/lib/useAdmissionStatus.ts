import { useEffect, useState } from "react";
import { fetchAdmissionStatus } from "./admission";
import type { AdmissionStatus } from "../admissionTypes";

/** Poll cadence while the gateway leg is still resolving (`gateway.state ===
 * "bootstrapping"`) - mirrors `lib/useCredentialsStatus.ts`'s identical
 * constant. */
const POLL_MS = 3_000;

/** Give up re-polling after this many attempts (~60s) - mirrors
 * `lib/useCredentialsStatus.ts`'s identical rationale. */
const MAX_POLLS = 20;

/**
 * Shared connection-state hook for the admission-gate Verify section:
 * fetches `admission_status` once, and re-polls every {@link POLL_MS} while
 * the gateway leg is still bootstrapping. Settles (stops polling) the
 * moment `gateway.state` is anything other than "bootstrapping" - the
 * verdryx binary/db legs and the drills scenario dir are re-checked fresh
 * on every fetch regardless (see `admission::env`'s own doc comment,
 * "Honest per-piece resolution states"), so there is nothing further to
 * poll for on their account.
 */
export function useAdmissionStatus(): AdmissionStatus | null {
  const [status, setStatus] = useState<AdmissionStatus | null>(null);

  useEffect(() => {
    let cancelled = false;
    let pollCount = 0;
    let timer: ReturnType<typeof setTimeout> | undefined;

    const tick = async () => {
      const next = await fetchAdmissionStatus();
      if (cancelled) return;
      setStatus(next);
      pollCount += 1;
      if (next.gateway.state === "bootstrapping" && pollCount < MAX_POLLS) {
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
