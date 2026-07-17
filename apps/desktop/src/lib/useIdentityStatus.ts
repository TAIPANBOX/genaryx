import { useEffect, useState } from "react";
import { fetchIdentityStatus } from "./identity";
import type { IdentityStatus } from "../identityTypes";

/** Poll cadence while the backend is still resolving (`state: "bootstrapping"`,
 * see `src-tauri/src/identity/state.rs`'s non-blocking bootstrap). Mirrors
 * `lib/usePolicyStatus.ts`'s identical constant. */
const POLL_MS = 3_000;

/** Give up re-polling after this many attempts (~60s) - mirrors
 * `lib/usePolicyStatus.ts`'s identical rationale. */
const MAX_POLLS = 20;

/**
 * Shared connection-state hook for the Identity view: fetches
 * `identity_status` once, and re-polls every [`POLL_MS`] while the backend
 * is still bootstrapping. Settles (stops polling) the moment the state is
 * anything other than "bootstrapping". Structurally identical to
 * `lib/usePolicyStatus.ts`'s `usePolicyStatus`, duplicated rather than
 * generalized over every panel since `IdentityStatus`/`PolicyStatus` are
 * independent Rust-side types that only happen to share this shape today.
 */
export function useIdentityStatus(): IdentityStatus | null {
  const [status, setStatus] = useState<IdentityStatus | null>(null);

  useEffect(() => {
    let cancelled = false;
    let pollCount = 0;
    let timer: ReturnType<typeof setTimeout> | undefined;

    const tick = async () => {
      const next = await fetchIdentityStatus();
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
