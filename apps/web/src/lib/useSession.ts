import { useEffect, useState } from "react";
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
 */
export function useSession(): Session | null {
  const [session, setSession] = useState<Session | null>(null);

  useEffect(() => {
    if (!isWebShell()) return;
    let cancelled = false;
    void fetchSession().then((s) => {
      if (!cancelled) setSession(s);
    });
    return () => {
      cancelled = true;
    };
  }, []);

  return session;
}
