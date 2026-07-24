import { useEffect, useState } from "react";
import { fetchPolicyStatus } from "./policy";
import type { PolicyStatus } from "../policyTypes";

/** Poll cadence while the backend is still resolving (`state: "bootstrapping"`,
 * see `crates/api/src/policy/state.rs`'s non-blocking bootstrap). Mirrors
 * `lib/useMoneyStatus.ts`'s identical constant. */
const POLL_MS = 3_000;

/** Give up re-polling after this many attempts (~60s) - mirrors
 * `lib/useMoneyStatus.ts`'s identical rationale. */
const MAX_POLLS = 20;

/**
 * Shared connection-state hook for the Policy view: fetches `policy_status`
 * once, and re-polls every [`POLL_MS`] while the backend is still
 * bootstrapping. Settles (stops polling) the moment the state is anything
 * other than "bootstrapping". Structurally identical to
 * `lib/useMoneyStatus.ts`'s `useMoneyStatus`, duplicated rather than
 * generalized over both panels since the two states
 * (`PolicyStatus`/`MoneyStatus`) are independent Rust-side types that only
 * happen to share this shape today.
 */
export function usePolicyStatus(): PolicyStatus | null {
  const [status, setStatus] = useState<PolicyStatus | null>(null);

  useEffect(() => {
    let cancelled = false;
    let pollCount = 0;
    let timer: ReturnType<typeof setTimeout> | undefined;

    const tick = async () => {
      const next = await fetchPolicyStatus();
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
