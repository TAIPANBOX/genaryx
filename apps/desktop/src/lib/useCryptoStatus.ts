import { useEffect, useState } from "react";
import { fetchCryptoStatus } from "./crypto";
import type { CryptoStatus } from "../cryptoTypes";

/** Poll cadence while the backend is still resolving - mirrors
 * `lib/useIdentityStatus.ts`'s identical constant. In practice
 * `crypto::state::bootstrap` never actually awaits anything (see its own
 * doc comment), so this settles on the very first tick; kept for
 * consistency with every other panel's status hook. */
const POLL_MS = 3_000;

/** Give up re-polling after this many attempts (~60s) - mirrors
 * `lib/useIdentityStatus.ts`'s identical rationale. */
const MAX_POLLS = 20;

/**
 * Shared connection-state hook for the Crypto view: fetches `crypto_status`
 * once, and re-polls every [`POLL_MS`] while the backend is still
 * bootstrapping. Structurally identical to `lib/useIdentityStatus.ts`'s
 * `useIdentityStatus` and `lib/useQualityStatus.ts`'s `useQualityStatus`,
 * duplicated rather than generalized since `CryptoStatus` is its own
 * independent Rust-side type.
 */
export function useCryptoStatus(): CryptoStatus | null {
  const [status, setStatus] = useState<CryptoStatus | null>(null);

  useEffect(() => {
    let cancelled = false;
    let pollCount = 0;
    let timer: ReturnType<typeof setTimeout> | undefined;

    const tick = async () => {
      const next = await fetchCryptoStatus();
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
