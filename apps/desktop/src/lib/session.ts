import { webApiBase, type ConsoleRole } from "./transport";

export type { ConsoleRole };

/** How the signed-in principal authenticated this session
 * (docs/CONSOLE-IDP.md): the box's own Argon2id account, or an offline-verified
 * OIDC ID token from the customer's IdP. */
export type ConsoleMethod = "local" | "oidc";

/**
 * The console session, as `GET /auth/session` reports it
 * (docs/CONSOLE-IDP.md's login contract). Shared by `WebGate.tsx` (the
 * sign-in gate) and anything past the gate that wants to know WHO is signed
 * in and with what privilege - `AppHeader.tsx`'s role badge, and the
 * Money/Policy views' 403 role-required message - rather than three
 * slightly-different copies of the same four-field GET.
 */
export interface Session {
  configured: boolean;
  signed_in: boolean;
  user: string | null;
  role: ConsoleRole | null;
  method: ConsoleMethod | null;
  /** Whether an OIDC config is present on this box, so the sign-in form
   * should offer the "Sign in with your organization" path. */
  oidc_available: boolean;
}

/** Raw fetch of `GET /auth/session` - no caching, no retry. Callers own their
 * own error/retry semantics: `WebGate.tsx`'s gate distinguishes "cannot reach
 * the box at all" from every other state, which is a different contract than
 * `useSession`'s plain "know the role, or don't" need in `lib/useSession.ts`. */
export async function fetchSession(): Promise<Session> {
  const resp = await fetch(`${webApiBase()}/auth/session`, { credentials: "include" });
  return (await resp.json()) as Session;
}
