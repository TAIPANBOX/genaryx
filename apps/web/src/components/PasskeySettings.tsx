import { useEffect, useState } from "react";
import { PopoverHeader } from "../lib/popover";
import {
  CeremonyCancelled,
  enrollPasskey,
  listPasskeys,
  operatorPasswordRequired,
  removePasskey,
  webauthnAvailable,
  type PasskeyInfo,
} from "../lib/webauthn";

/** `created_at` is an RFC 3339 timestamp
 * (`crates/web/src/webauthn.rs`'s `PasskeyRecord.created_at`); shown as a
 * plain local date, falling back to the raw string rather than throwing if
 * it ever fails to parse. */
function createdLabel(createdAt: string): string {
  const ms = Date.parse(createdAt);
  return Number.isFinite(ms) ? new Date(ms).toLocaleDateString() : createdAt;
}

/** Read a rejection's own text, whatever shape it arrived in: an `Error`'s
 * `message`, a raw `{error: string}` REST body (`lib/webauthn.ts`'s
 * `enrollPasskey`/`listPasskeys` reject with the server's body unmodified),
 * or a last-resort stringify. */
function errorText(err: unknown): string {
  if (err instanceof Error) return err.message;
  if (err && typeof err === "object" && typeof (err as { error?: unknown }).error === "string") {
    return (err as { error: string }).error;
  }
  return String(err);
}

/**
 * The Passkeys panel (D15 B3/2, docs/CONSOLE-IDP.md): every passkey enrolled
 * for the signed-in operator, plus the one control that adds one. Opened
 * from `AppHeader`'s session area as a popover window (`usePopover`),
 * matching every other detail card in this app (`UserCard`,
 * `AgentDetailCard`, ...) rather than a bespoke modal - this is a settings
 * surface, not the loud break-glass ceremony `BreakGlassDialog` is reserved
 * for.
 *
 * Three honest states, never a fabricated one:
 * - `!webauthnAvailable()`: this browser/origin cannot run WebAuthn at all
 *   (`lib/webauthn.ts`'s own doc explains exactly why - not a secure
 *   context). Enrolling is impossible here, and the panel says so instead
 *   of showing a button that would only fail.
 * - No passkeys enrolled: privileged actions (kill, budget, approval) still
 *   go through, journaled software-signed (the documented trial fallback,
 *   `crates/web/src/main.rs`'s `webauthn_gate`) - enrolling the first one
 *   is what upgrades them to hardware-confirmed.
 * - One or more enrolled: the ceremony is REQUIRED from here on
 *   (`GET /api/webauthn/passkeys`'s own `webauthn_required` flag mirrors
 *   this exactly), so the list reads as what is already protecting those
 *   actions, not as a suggestion.
 *
 * A plain operator cancel of the platform's own passkey prompt
 * ({@link CeremonyCancelled}) is treated as "say nothing" - never an error
 * banner for what is simply "not right now".
 */
export function PasskeySettings() {
  const [passkeys, setPasskeys] = useState<PasskeyInfo[] | null | undefined>(undefined);
  const [policyRequires, setPolicyRequires] = useState(false);
  const [label, setLabel] = useState("");
  const [enrolling, setEnrolling] = useState(false);
  const [removing, setRemoving] = useState<string | null>(null);
  const [password, setPassword] = useState("");
  const [needPassword, setNeedPassword] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const refresh = () => {
    void listPasskeys()
      .then((probe) => {
        setPasskeys(probe.passkeys);
        setPolicyRequires(probe.policy_requires_passkey);
      })
      .catch((err: unknown) => {
        setPasskeys(null);
        setError(errorText(err));
      });
  };

  // Runs once per mount - the popover is closed and reopened (a fresh
  // component instance) rather than left mounted in the background, so
  // there is no separate "went stale while closed" case to handle here.
  useEffect(() => {
    refresh();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const available = webauthnAvailable();

  async function onEnroll() {
    setEnrolling(true);
    setError(null);
    try {
      await enrollPasskey(label.trim());
      setLabel("");
      refresh();
    } catch (err) {
      if (!(err instanceof CeremonyCancelled)) setError(errorText(err));
    } finally {
      setEnrolling(false);
    }
  }

  /** Remove one enrolled passkey. Two proofs, decided by the server and not
   * guessed here: an assertion from an enrolled key for any but the last, the
   * operator password for the last one (`lib/webauthn.ts`'s `removePasskey`).
   * A `password_required` refusal is not an error to show and forget, it is
   * the server asking for the field below, so it puts it on screen. */
  async function onRemove(credentialId: string) {
    setRemoving(credentialId);
    setError(null);
    try {
      await removePasskey(credentialId, password.trim() || undefined);
      setPassword("");
      setNeedPassword(false);
      refresh();
    } catch (err) {
      if (err instanceof CeremonyCancelled) return;
      if (operatorPasswordRequired(err)) setNeedPassword(true);
      setError(errorText(err));
    } finally {
      setRemoving(null);
    }
  }

  return (
    <div className="flex flex-col">
      <PopoverHeader kicker="Session" title="Passkeys" />
      <div className="flex flex-col gap-3" style={{ padding: "0 16px 16px" }}>
        {!available && (
          <div className="text-[11.5px]" style={{ color: "var(--sev-medium)", lineHeight: 1.6 }}>
            This page is not a secure context, so the browser has no WebAuthn here. Reach the
            console as <code className="mono">localhost</code> (loopback, or an{" "}
            <code className="mono">ssh -L</code> forward over your tunnel) or behind TLS to
            enroll or use a passkey.
          </div>
        )}

        {passkeys === undefined && (
          <div className="text-[12px]" style={{ color: "var(--faint)" }}>
            loading...
          </div>
        )}

        {passkeys && passkeys.length === 0 && !policyRequires && (
          <div className="text-[11.5px]" style={{ color: "var(--dim)", lineHeight: 1.6 }}>
            No passkey enrolled yet. Kill / budget / approval / tunnel actions still go through,
            journaled as software-signed. Enrolling a passkey upgrades those actions to
            hardware-confirmed.
          </div>
        )}

        {passkeys && passkeys.length === 0 && policyRequires && (
          <div className="text-[11.5px]" style={{ color: "var(--sev-high)", lineHeight: 1.6 }}>
            No passkey enrolled, and this console requires one (
            <code className="mono">GENARYX_WEB_REQUIRE_PASSKEY</code>). Kill / budget / approval /
            tunnel actions are refused until you enrol one here.
          </div>
        )}

        {passkeys && passkeys.length > 0 && (
          <div className="flex flex-col">
            {passkeys.map((k) => (
              <div
                key={k.credential_id}
                className="flex items-center justify-between gap-3"
                style={{ padding: "6px 0", borderBottom: "1px solid var(--line)" }}
              >
                <span className="text-[12px] truncate" style={{ color: "var(--fg)" }}>
                  {k.label}
                </span>
                <span className="flex items-center gap-3">
                  <span className="mono text-[11px]" style={{ color: "var(--faint)" }}>
                    {createdLabel(k.created_at)}
                  </span>
                  <button
                    type="button"
                    className="icon-btn"
                    style={{ width: "auto", padding: "0 8px", fontSize: 11, whiteSpace: "nowrap" }}
                    onClick={() => void onRemove(k.credential_id)}
                    disabled={removing !== null}
                    title="Remove this passkey"
                  >
                    {removing === k.credential_id ? "Removing..." : "Remove"}
                  </button>
                </span>
              </div>
            ))}
          </div>
        )}

        {needPassword && (
          <div className="flex flex-col gap-2">
            <div className="text-[11.5px]" style={{ color: "var(--dim)", lineHeight: 1.6 }}>
              This is the last enrolled passkey. Removing it takes this console back to
              session-only, so it needs the operator password (the one{" "}
              <code className="mono">genaryx-web set-password</code> set), not a passkey.
            </div>
            <input
              className="mono"
              type="password"
              style={{
                background: "var(--panel)",
                border: "1px solid var(--line-2)",
                borderRadius: 8,
                padding: "6px 10px",
                fontSize: 12,
                color: "var(--fg)",
              }}
              placeholder="operator password"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              disabled={removing !== null || enrolling}
              spellCheck={false}
            />
          </div>
        )}

        {available && (
          <div className="flex items-center gap-2">
            <input
              className="mono"
              style={{
                flex: 1,
                minWidth: 0,
                background: "var(--panel)",
                border: "1px solid var(--line-2)",
                borderRadius: 8,
                padding: "6px 10px",
                fontSize: 12,
                color: "var(--fg)",
              }}
              placeholder="label (optional)"
              value={label}
              onChange={(e) => setLabel(e.target.value)}
              disabled={enrolling}
              spellCheck={false}
            />
            <button
              type="button"
              className="icon-btn"
              style={{ width: "auto", padding: "0 12px", fontSize: 11, whiteSpace: "nowrap" }}
              onClick={() => void onEnroll()}
              disabled={enrolling}
            >
              {enrolling ? "Confirming..." : "Add passkey"}
            </button>
          </div>
        )}

        {error && (
          <div
            className="panel px-3 py-2 mono text-[11.5px]"
            style={{ background: "var(--panel-2)", color: "var(--sev-high)" }}
          >
            {error}
          </div>
        )}
      </div>
    </div>
  );
}
