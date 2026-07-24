import { useCallback, useEffect, useState } from "react";
import { fetchPocketStatus } from "./pocket";
import type { PocketStatus } from "../pocketTypes";

/** Poll cadence while `watching` (an armed QR is on screen, waiting for the
 * phone to pair) - fast enough that the panel flips to Paired within a
 * couple seconds of a real pairing, not so fast it hammers the relay's
 * admin API over a background WG link. */
const WATCH_POLL_MS = 2_000;

/**
 * Pocket panel status hook: fetches `pocket_status` once on mount (and
 * again whenever `refresh()` is called), and, while `watching` is true,
 * re-polls every [`WATCH_POLL_MS`] so the panel notices the phone pairing
 * and flips to the Paired view on its own - the operator never has to
 * manually refresh mid-scan. Polling stops the instant `watching` turns
 * false; there is no background poll loop once nothing is being waited on.
 *
 * `PocketView` applies `pocketConnect()`/`pocketDisconnect()`'s own return
 * values to local state directly rather than calling `refresh()` right
 * after (both already return the outcome that matters), so `refresh()`
 * mainly exists for an explicit operator retry after an error.
 */
export function usePocketStatus(watching: boolean): {
  status: PocketStatus | null;
  refresh: () => void;
} {
  const [status, setStatus] = useState<PocketStatus | null>(null);
  const [refreshNonce, setRefreshNonce] = useState(0);
  const refresh = useCallback(() => setRefreshNonce((n) => n + 1), []);

  useEffect(() => {
    let cancelled = false;
    void fetchPocketStatus().then((next) => {
      if (!cancelled) setStatus(next);
    });
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [refreshNonce]);

  useEffect(() => {
    if (!watching) return;
    let cancelled = false;
    const timer = setInterval(() => {
      void fetchPocketStatus().then((next) => {
        if (!cancelled) setStatus(next);
      });
    }, WATCH_POLL_MS);
    return () => {
      cancelled = true;
      clearInterval(timer);
    };
  }, [watching]);

  return { status, refresh };
}
