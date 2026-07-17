import { useEffect, useState } from "react";
import { fetchRemoteStatus } from "./remote";
import type { RemoteStatus } from "../remoteTypes";

/** Poll cadence while the backend is still resolving - mirrors
 * `lib/useEvidenceStatus.ts`'s identical constant. In practice
 * `remote::state::bootstrap` never actually awaits anything (one cheap
 * best-effort filesystem/PATH check - see its own doc comment), so this
 * settles on the very first tick; kept for consistency with every other
 * panel's status hook. */
const POLL_MS = 3_000;

/** Give up re-polling after this many attempts (~60s) - mirrors
 * `lib/useEvidenceStatus.ts`'s identical rationale. */
const MAX_POLLS = 20;

/**
 * Shared connection-state hook for the Remote view: fetches `remote_status`
 * once, and re-polls every [`POLL_MS`] while the backend is still
 * bootstrapping. Structurally identical to `lib/useEvidenceStatus.ts`'s
 * `useEvidenceStatus`, duplicated rather than generalized since `RemoteStatus`
 * is its own independent Rust-side type.
 *
 * `RemoteView` seeds its OWN local status state from this hook's first
 * value, then updates it directly from every mutating action's own return
 * (every `remote_*` mutator already returns the fresh whole-panel status -
 * see `lib/remote.ts`) rather than re-polling after every click.
 */
export function useRemoteStatus(): RemoteStatus | null {
  const [status, setStatus] = useState<RemoteStatus | null>(null);

  useEffect(() => {
    let cancelled = false;
    let pollCount = 0;
    let timer: ReturnType<typeof setTimeout> | undefined;

    const tick = async () => {
      const next = await fetchRemoteStatus();
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
