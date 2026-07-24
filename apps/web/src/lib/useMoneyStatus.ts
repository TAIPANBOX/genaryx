import { useEffect, useState } from "react";
import { fetchMoneyStatus } from "./money";
import type { MoneyStatus } from "../moneyTypes";

/** Poll cadence while the backend is still pairing (`state: "bootstrapping"`,
 * see `crates/api/src/money/state.rs`'s non-blocking bootstrap). */
const POLL_MS = 3_000;

/** Give up re-polling after this many attempts (~60s) so a genuinely stuck
 * backend does not poll forever; the last-seen status (still
 * "bootstrapping") just stays on screen rather than spinning silently. */
const MAX_POLLS = 20;

/**
 * Shared connection-state hook for the Overview and Money views: fetches
 * `money_status` once, and re-polls every [`POLL_MS`] while the backend is
 * still bootstrapping (see `state.rs`'s module docs for why bootstrap is
 * async/non-blocking - the frontend has to poll past that window rather
 * than assuming a synchronous answer). Settles (stops polling) the moment
 * the state is anything other than "bootstrapping".
 */
export function useMoneyStatus(): MoneyStatus | null {
  const [status, setStatus] = useState<MoneyStatus | null>(null);

  useEffect(() => {
    let cancelled = false;
    let pollCount = 0;
    let timer: ReturnType<typeof setTimeout> | undefined;

    const tick = async () => {
      const next = await fetchMoneyStatus();
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
