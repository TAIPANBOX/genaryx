import { useState, type FormEvent } from "react";

/**
 * Demo funnel step 1, "Sign in": a look-alike of the console's own
 * `WebGate`/`SignIn` card (`components/WebGate.tsx`), same card chrome, same
 * field layout, same heading and intro copy, so the demo genuinely shows
 * what the real gate looks like. Unlike the real thing, nothing here is
 * wired to a backend: the fields are pre-filled and never validated,
 * clicking "Sign in" always advances (after a short simulated delay, the
 * same "Signing in..." beat the real gate has).
 *
 * `autoComplete="off"` on both fields is a deliberate divergence from the
 * real gate (which reasonably uses `username`/`current-password` hints):
 * this form is not a real login, and inviting a visitor's browser to offer
 * up their own saved credentials, or to save this dummy pair, would be a
 * bad outcome for a public demo.
 *
 * No dead end: the one primary button is the only way forward.
 */
export function SignInStep({ onSignIn }: { onSignIn: () => void }) {
  const [username, setUsername] = useState("ops");
  const [password, setPassword] = useState("demo-pw-2026");
  const [busy, setBusy] = useState(false);

  function submit(e: FormEvent) {
    e.preventDefault();
    if (busy) return;
    setBusy(true);
    // Nothing to await, this only mirrors the real gate's own brief
    // "Signing in..." beat rather than snapping straight through.
    setTimeout(onSignIn, 450);
  }

  return (
    <div className="flex min-h-screen items-center justify-center bg-neutral-950 p-6 text-neutral-100">
      <div className="w-full max-w-sm rounded-2xl border border-neutral-800 bg-neutral-900/60 p-7">
        <div className="flex items-center gap-2">
          <svg viewBox="0 0 24 24" width="16" height="16" fill="none" aria-hidden="true">
            <path d="M13.5 2 5 13.2h5.1L9.4 22l9-11.8h-5.3L13.5 2Z" fill="#f4b23e" />
          </svg>
          <span className="text-xs uppercase tracking-widest text-neutral-500">Genaryx</span>
        </div>
        <h1 className="mt-2 text-xl font-semibold">Sign in to your console</h1>
        <p className="mt-2 text-sm leading-relaxed text-neutral-400">
          This console runs on your own box. Your runs, spend and identities never leave it.
        </p>

        <form onSubmit={submit}>
          <label className="mt-6 block text-sm text-neutral-300" htmlFor="demo-gx-user">
            Operator
          </label>
          <input
            id="demo-gx-user"
            className="mt-1 w-full rounded-lg border border-neutral-700 bg-neutral-950 px-3 py-2 text-sm outline-none focus:border-neutral-500"
            value={username}
            onChange={(e) => setUsername(e.target.value)}
            autoComplete="off"
          />

          <label className="mt-4 block text-sm text-neutral-300" htmlFor="demo-gx-pass">
            Password
          </label>
          <input
            id="demo-gx-pass"
            type="password"
            className="mt-1 w-full rounded-lg border border-neutral-700 bg-neutral-950 px-3 py-2 text-sm outline-none focus:border-neutral-500"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            autoComplete="off"
          />

          <button
            type="submit"
            disabled={busy}
            className="mt-6 w-full rounded-lg bg-neutral-100 px-3 py-2 text-sm font-semibold text-neutral-900 disabled:opacity-40"
          >
            {busy ? "Signing in" : "Sign in"}
          </button>
        </form>

        <p className="mt-5 text-center text-[11px] text-neutral-600">
          Demo, simulated data, nothing leaves your browser.
        </p>
      </div>
    </div>
  );
}
