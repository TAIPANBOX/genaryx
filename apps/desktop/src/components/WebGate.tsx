import { useCallback, useEffect, useState } from "react";
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
 */
type Session = {
  configured: boolean;
  signed_in: boolean;
  user: string | null;
};

export function WebGate({ children }: { children: React.ReactNode }) {
  const [session, setSession] = useState<Session | null>(null);
  const [failed, setFailed] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      const resp = await fetch(`${webApiBase()}/auth/session`, {
        credentials: "include",
      });
      setSession((await resp.json()) as Session);
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
  if (!session.signed_in) return <SignIn onDone={refresh} />;
  return <>{children}</>;
}

function SignIn({ onDone }: { onDone: () => void }) {
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
    <div className="flex min-h-screen items-center justify-center bg-neutral-950 p-6 text-neutral-100">
      <form
        onSubmit={submit}
        className="w-full max-w-sm rounded-2xl border border-neutral-800 bg-neutral-900/60 p-7"
      >
        <div className="text-xs uppercase tracking-widest text-neutral-500">Genaryx</div>
        <h1 className="mt-2 text-xl font-semibold">Sign in to your console</h1>
        <p className="mt-2 text-sm leading-relaxed text-neutral-400">
          This console runs on your own box. Your runs, spend and identities never leave it.
        </p>

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
    </div>
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
