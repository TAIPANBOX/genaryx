import { useEffect, useState } from "react";
import { fetchCredentialsStatus } from "./credentials";
import type { CredentialsStatus } from "./credentials";

/** Poll cadence while the backend is still resolving (`state:
 * "bootstrapping"`) - mirrors `lib/useIdentityStatus.ts`'s identical
 * constant. */
const POLL_MS = 3_000;

/** Give up re-polling after this many attempts (~60s) - mirrors
 * `lib/useIdentityStatus.ts`'s identical rationale. */
const MAX_POLLS = 20;

/**
 * Shared connection-state hook for the Credentials card: fetches
 * `credentials_status` once, and re-polls every {@link POLL_MS} while the
 * backend is still bootstrapping. Settles (stops polling) the moment the
 * state is anything other than "bootstrapping". Structurally identical to
 * `lib/useIdentityStatus.ts`'s `useIdentityStatus` - duplicated rather than
 * generalized, same rationale that hook's own doc comment gives: independent
 * Rust-side types that only happen to share this shape.
 *
 * Deliberately independent of `useIdentityStatus`: the Credentials plane
 * resolves a different descriptor service (`services.gateway`, not
 * `services.idryx`) and can be `ready` while Identity is `no_environment`
 * (the common case: an environment brought up without `--with idryx`), or
 * vice versa.
 */
export function useCredentialsStatus(): CredentialsStatus | null {
  const [status, setStatus] = useState<CredentialsStatus | null>(null);

  useEffect(() => {
    let cancelled = false;
    let pollCount = 0;
    let timer: ReturnType<typeof setTimeout> | undefined;

    const tick = async () => {
      const next = await fetchCredentialsStatus();
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
