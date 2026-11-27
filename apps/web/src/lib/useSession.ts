import { useEffect, useState } from "react";
import { DEMO_SESSION, MOCK } from "./mockPreview";
import { fetchSession, type Session } from "./session";
import { isWebShell } from "./transport";

/**
 * The signed-in console session, for anything past `WebGate` that wants to
 * know who is signed in and with what role - `AppHeader.tsx`'s role badge,
 * and the Money/Policy views' 403 role-required message ("you are signed in
 * as ..."). Fetched once on mount, independently of `WebGate`'s own session
 * state - no global store, matching every other plane's `useXStatus` hook
 * (`useMoneyStatus.ts`, `usePolicyStatus.ts`, ...); a second `GET
 * /auth/session` is a cheap, read-only cookie check, not a real cost.
 *
 * Always `null` in the desktop shell (`isWebShell()` false): there is no
 * console session there, matching `WebGate.tsx`'s own gate bypass for that
 * shell.
 *
 * The Live Demo (`MOCK`) answers a fixed operator from the first render, so
 * the sidebar foot shows the same session badge the real console shows. It
 * asks no box: the demo build has none, and its network calls are blocked.
 */
export function useSession(): Session | null {
  const [session, setSession] = useState<Session | null>(() => (MOCK ? DEMO_SESSION : null));

  useEffect(() => {
    if (MOCK) return;
    if (!isWebShell()) return;
    let cancelled = false;
    // A box that cannot be reached (network down, a non-JSON answer) leaves
    // no session rather than an unhandled rejection: the badge simply does
    // not appear, as for a signed-out user.
    fetchSession().then(
      (s) => {
        if (!cancelled) setSession(s);
      },
      () => {
        if (!cancelled) setSession(null);
      },
    );
    return () => {
      cancelled = true;
    };
  }, []);

  return session;
}
