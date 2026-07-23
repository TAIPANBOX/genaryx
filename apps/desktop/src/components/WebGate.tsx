import { useCallback, useEffect, useState } from "react";
import { fetchSession, type Session } from "../lib/session";
import { isWebShell, webApiBase } from "../lib/transport";

/**
 * The sign-in gate, and only in the browser build.
 *
 * The desktop shell is already running on the operator's machine beside the
 * stack, so it renders straight through. The web build is reached over the
 * tunnel from some other device, so it asks who you are first.
 *
 * What signing in does NOT get you is worth stating, because the UI should
 * not imply otherwise: a session opens the console, it does not authorise a
 * destructive action. Killing a run or moving a budget is re-signed at the
 * moment it happens (D11/D13), so a stolen session can look, not act.
 *
 * Two ways in (docs/CONSOLE-IDP.md): the box's own local account, always
 * available, or - when `session.oidc_available` says the box has a JWKS
 * configured - pasting an OIDC ID token issued by the operator's own IdP.
 * There is no OAuth redirect here: the box has no browser round-trip to the
 * IdP, so the operator brings the token themselves, and it is verified
 * offline, on this box, against the configured JWKS.
 */
export function WebGate({ children }: { children: React.ReactNode }) {
  const [session, setSession] = useState<Session | null>(null);
  const [failed, setFailed] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      setSession(await fetchSession());
      setFailed(null);
    } catch {
      // Not signed out: unreachable. Nearly always the tunnel is down, and
      // saying "wrong password" here would send the operator hunting for the
      // wrong problem.
      setFailed("Cannot reach the console on this box. Is the tunnel up?");
    }
  }, []);

  useEffect(() => {
    if (isWebShell()) void refresh();
  }, [refresh]);

  if (!isWebShell()) return <>{children}</>;
  if (failed) return <Notice title="No answer from the box" body={failed} retry={refresh} />;
  if (!session) return <Notice title="Connecting" body="Reaching the console on your box." />;
  if (!session.configured) {
    return (
      <Notice
        title="This box has no operator yet"
        body="Set one on the box itself, then sign in here:"
        code="genaryx-web set-password --username you"
      />
    );
  }
  if (!session.signed_in) return <SignIn session={session} onDone={refresh} />;
  return <>{children}</>;
}

/**
 * The card chrome (brand mark, heading, intro line) is shared by both ways
 * in; which forms it holds depends on `session.oidc_available` - the local
 * form always renders, the "Sign in with your organization" block only when
 * this box has a JWKS configured (docs/CONSOLE-IDP.md).
 */
function SignIn({ session, onDone }: { session: Session; onDone: () => void }) {
  return (
    <div className="flex min-h-screen items-center justify-center bg-neutral-950 p-6 text-neutral-100">
      <div className="w-full max-w-sm rounded-2xl border border-neutral-800 bg-neutral-900/60 p-7">
        <div className="text-xs uppercase tracking-widest text-neutral-500">Genaryx</div>
        <h1 className="mt-2 text-xl font-semibold">Sign in to your console</h1>
        <p className="mt-2 text-sm leading-relaxed text-neutral-400">
          This console runs on your own box. Your runs, spend and identities never leave it.
        </p>

        <LocalSignIn onDone={onDone} />

        {session.oidc_available && (
          <>
            <div className="mt-7 flex items-center gap-3 text-xs uppercase tracking-widest text-neutral-600">
              <div className="h-px flex-1 bg-neutral-800" aria-hidden="true" />
              or
              <div className="h-px flex-1 bg-neutral-800" aria-hidden="true" />
            </div>
            <OidcSignIn onDone={onDone} />
          </>
        )}
      </div>
    </div>
  );
}

/** The box's own Argon2id account - always available, and the break-glass
 * owner even when OIDC is also configured. Unchanged from before OIDC login
 * existed, just extracted out of `SignIn` so the card can hold a second,
 * clearly-separated form underneath it. */
function LocalSignIn({ onDone }: { onDone: () => void }) {
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    setBusy(true);
    setError(null);
    try {
      const resp = await fetch(`${webApiBase()}/auth/login`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        credentials: "include",
        body: JSON.stringify({ username, password }),
      });
      if (!resp.ok) {
        const body = (await resp.json().catch(() => null)) as { error?: string } | null;
        setError(body?.error ?? "Sign-in refused.");
        return;
      }
      onDone();
    } catch {
      setError("Cannot reach the console on this box.");
    } finally {
      setBusy(false);
    }
  }

  return (
    <form onSubmit={submit}>
      <label className="mt-6 block text-sm text-neutral-300" htmlFor="gx-user">
        Operator
      </label>
      <input
        id="gx-user"
        className="mt-1 w-full rounded-lg border border-neutral-700 bg-neutral-950 px-3 py-2 text-sm outline-none focus:border-neutral-500"
        value={username}
        onChange={(e) => setUsername(e.target.value)}
        autoComplete="username"
        autoFocus
      />

      <label className="mt-4 block text-sm text-neutral-300" htmlFor="gx-pass">
        Password
      </label>
      <input
        id="gx-pass"
        type="password"
        className="mt-1 w-full rounded-lg border border-neutral-700 bg-neutral-950 px-3 py-2 text-sm outline-none focus:border-neutral-500"
        value={password}
        onChange={(e) => setPassword(e.target.value)}
        autoComplete="current-password"
      />

      {error && (
        <div className="mt-4 rounded-lg border border-red-900/60 bg-red-950/40 px-3 py-2 text-sm text-red-200">
          {error}
        </div>
      )}

      <button
        type="submit"
        disabled={busy || !username || !password}
        className="mt-6 w-full rounded-lg bg-neutral-100 px-3 py-2 text-sm font-semibold text-neutral-900 disabled:opacity-40"
      >
        {busy ? "Signing in" : "Sign in"}
      </button>
    </form>
  );
}

/**
 * "Sign in with your organization" (docs/CONSOLE-IDP.md): the operator pastes
 * an OIDC ID token their IdP already issued them, and this posts it straight
 * to `POST /auth/login` as `{ id_token }`. Deliberately NOT an OAuth redirect
 * flow - this box has no browser round-trip to the IdP to receive one, so the
 * operator brings the token themselves, the same shape tokenfuse's own
 * offline OIDC login already uses.
 */
function OidcSignIn({ onDone }: { onDone: () => void }) {
  const [token, setToken] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    setBusy(true);
    setError(null);
    try {
      const resp = await fetch(`${webApiBase()}/auth/login`, {
        method: "POST",
        headers: { "content-type": "application/json" },
        credentials: "include",
        body: JSON.stringify({ id_token: token.trim() }),
      });
      if (!resp.ok) {
        const body = (await resp.json().catch(() => null)) as { error?: string } | null;
        setError(body?.error ?? "Sign-in refused.");
        return;
      }
      onDone();
    } catch {
      setError("Cannot reach the console on this box.");
    } finally {
      setBusy(false);
    }
  }

  return (
    <form onSubmit={submit} className="mt-6">
      <div className="text-sm font-medium text-neutral-200">Sign in with your organization</div>
      <p className="mt-1.5 text-xs leading-relaxed text-neutral-500">
        This token is verified right here on this box, against the JWKS your organization
        configured - it never leaves the box and is never stored. The local account above stays
        the break-glass owner either way.
      </p>

      <label className="mt-3 block text-sm text-neutral-300" htmlFor="gx-token">
        ID token (JWT)
      </label>
      <textarea
        id="gx-token"
        rows={3}
        className="mt-1 w-full rounded-lg border border-neutral-700 bg-neutral-950 px-3 py-2 font-mono text-xs leading-relaxed outline-none focus:border-neutral-500"
        value={token}
        onChange={(e) => setToken(e.target.value)}
        placeholder="eyJhbGciOi..."
        spellCheck={false}
        autoComplete="off"
      />

      {error && (
        <div className="mt-3 rounded-lg border border-red-900/60 bg-red-950/40 px-3 py-2 text-sm text-red-200">
          {error}
        </div>
      )}

      <button
        type="submit"
        disabled={busy || !token.trim()}
        className="mt-4 w-full rounded-lg border border-neutral-700 px-3 py-2 text-sm font-semibold text-neutral-100 disabled:opacity-40"
      >
        {busy ? "Signing in" : "Sign in with token"}
      </button>
    </form>
  );
}

function Notice({
  title,
  body,
  code,
  retry,
}: {
  title: string;
  body: string;
  code?: string;
  retry?: () => void;
}) {
  return (
    <div className="flex min-h-screen items-center justify-center bg-neutral-950 p-6 text-neutral-100">
      <div className="w-full max-w-md rounded-2xl border border-neutral-800 bg-neutral-900/60 p-7">
        <h1 className="text-lg font-semibold">{title}</h1>
        <p className="mt-2 text-sm leading-relaxed text-neutral-400">{body}</p>
        {code && (
          <pre className="mt-3 overflow-x-auto rounded-lg border border-neutral-800 bg-neutral-950 px-3 py-2 text-xs text-neutral-300">
            {code}
          </pre>
        )}
        {retry && (
          <button
            onClick={retry}
            className="mt-5 rounded-lg border border-neutral-700 px-3 py-1.5 text-sm"
          >
            Try again
          </button>
        )}
      </div>
    </div>
  );
}
