import { useCallback, useEffect, useState } from "react";
import { cssVar } from "../lib/cssVars";
import { formatTimestamp } from "../lib/format";
import { describePocketError, pocketConnect, pocketDisconnect } from "../lib/pocket";
import { usePocketStatus } from "../lib/usePocketStatus";
import type { PocketError, PocketQr, PocketStatus } from "../pocketTypes";
import { QrCode } from "./QrCode";

const CARD_STYLE = { maxWidth: 480 } as const;
const ERROR_BANNER_STYLE = {
  color: "var(--sev-high)",
  background: "var(--panel)",
  borderRadius: 8,
  border: "1px solid var(--line)",
} as const;

function unixToTimestamp(unixSecs: number): string {
  return formatTimestamp(new Date(unixSecs * 1000).toISOString());
}

function secondsRemaining(expiresUnix: number, nowMs: number): number {
  return Math.max(0, expiresUnix - Math.floor(nowMs / 1000));
}

function statusDotColor(state: PocketStatus["state"]): string {
  switch (state) {
    case "paired":
      return "var(--sev-low)";
    case "idle":
      return "var(--faint)";
    case "relay_unreachable":
      return "var(--sev-high)";
  }
}

function statusLabel(status: PocketStatus): string {
  switch (status.state) {
    case "idle":
      return status.cloud_ready ? "no phone paired" : "no phone paired · Cloud not resolvable";
    case "paired":
      return `paired · ${status.name || status.device_id}`;
    case "relay_unreachable":
      return "relay unreachable";
  }
}

/**
 * The Pocket panel (docs/PHASE5.md W2, itrat-console/13 D12.2a): "Connect
 * TokenFuse Pocket" mints a pairing code at the Cloud, arms the relay's
 * pairing window, and renders the QR the phone scans - a later wave (W3)
 * builds the scanner itself. Three states: idle (Connect button),
 * showing-QR (an armed window, waiting for the phone), and paired (device
 * details + Disconnect).
 *
 * The "showing-QR" step is tracked entirely in THIS component's own `qr`
 * state, not the backend (`usePocketStatus`'s doc comment explains why: the
 * relay exposes no "is a window currently armed" read, only device-paired
 * state) - `qr !== null` is what drives `usePocketStatus`'s fast poll, so
 * the panel notices the phone pairing and flips to the Paired view on its
 * own, without the operator refreshing anything.
 */
export function PocketView() {
  const [qr, setQr] = useState<PocketQr | null>(null);
  const [now, setNow] = useState(() => Date.now());
  const [connecting, setConnecting] = useState(false);
  const [disconnecting, setDisconnecting] = useState(false);
  const [error, setError] = useState<PocketError | null>(null);

  const remaining = qr !== null ? secondsRemaining(qr.expires_unix, now) : 0;
  const watching = qr !== null && remaining > 0;
  const { status, refresh } = usePocketStatus(watching);

  // Redraws the countdown every second while a QR is on screen - the QR
  // itself never re-renders (its `pathData` only depends on `qr.qr_content`,
  // which does not change), only the "expires in Xs" text below it does.
  useEffect(() => {
    if (qr === null) return;
    const timer = setInterval(() => setNow(Date.now()), 1_000);
    return () => clearInterval(timer);
  }, [qr]);

  // The moment the backend reports paired (the phone scanned and redeemed
  // the code), drop the local QR state - rendering then falls through to
  // the Paired branch on `status` alone.
  useEffect(() => {
    if (status?.state === "paired" && qr !== null) setQr(null);
  }, [status, qr]);

  const onConnect = useCallback(async () => {
    if (connecting) return;
    setConnecting(true);
    setError(null);
    try {
      const result = await pocketConnect();
      setQr(result);
    } catch (err) {
      const e = err as PocketError;
      if (e.kind === "device_exists") {
        // Someone paired between the last status poll and this click - the
        // refresh below picks up the real Paired state; this is a normal
        // race, not a failure worth an error banner.
        refresh();
      } else {
        setError(e);
      }
    } finally {
      setConnecting(false);
    }
  }, [connecting, refresh]);

  const onCancel = useCallback(() => {
    // "Cancel" reuses Disconnect: nothing is actually paired yet at this
    // point in the flow, so it only closes the just-armed window
    // (`RelayAdminClient::disconnect` clears the pairing-window row
    // regardless of whether a device is paired, `registry.rs::disconnect`) -
    // an operator-initiated version of docs/PHASE5.md W2's "do not leave a
    // half-armed window silently" rule, not just the error-path cleanup
    // `pocket_connect` itself already does server-side.
    setQr(null);
    void pocketDisconnect()
      .catch(() => {
        // Best-effort - the window's own TTL closes it regardless.
      })
      .then(() => refresh());
  }, [refresh]);

  const onDisconnect = useCallback(async () => {
    if (disconnecting) return;
    setDisconnecting(true);
    setError(null);
    try {
      await pocketDisconnect();
    } catch (err) {
      setError(err as PocketError);
    } finally {
      setDisconnecting(false);
    }
    refresh();
  }, [disconnecting, refresh]);

  if (status === null) {
    return (
      <div className="flex-1 min-h-0 flex items-center justify-center">
        <div className="mono text-[12px]" style={{ color: "var(--faint)" }}>
          resolving the Pocket panel...
        </div>
      </div>
    );
  }

  return (
    <div className="flex-1 min-h-0 overflow-y-auto thin-scroll px-5 py-4 flex flex-col gap-4">
      <div className="flex flex-wrap items-center gap-2">
        <span className="chip" style={cssVar("dot", statusDotColor(status.state))}>
          <span className="dot" aria-hidden="true" />
          {statusLabel(status)}
        </span>
      </div>

      <div className="d-card px-4 py-3 flex flex-col gap-3" style={CARD_STYLE}>
        <span className="text-[11.5px]" style={{ color: "var(--dim)" }}>
          Pair your phone (TokenFuse Pocket) to this box&apos;s relay so you can see the exception queue
          and slide-to-kill a runaway from anywhere - a QR carries the relay&apos;s pinned TLS identity
          plus a one-time code, scanned once, no manual entry.
        </span>

        {status.state === "relay_unreachable" && (
          <div className="mono text-[11.5px] px-3 py-2" style={ERROR_BANNER_STYLE}>
            relay admin API unreachable - {status.message}
          </div>
        )}

        {status.state === "idle" && qr === null && (
          <div className="flex flex-col gap-2">
            <button
              type="button"
              className="icon-btn"
              style={{ width: "auto", padding: "0 14px", fontSize: 11, alignSelf: "flex-start" }}
              onClick={() => void onConnect()}
              disabled={connecting || !status.cloud_ready}
            >
              {connecting ? "Connecting..." : "Connect TokenFuse Pocket"}
            </button>
            {!status.cloud_ready && (
              <span className="text-[11px]" style={{ color: "var(--faint)" }}>
                no TokenFuse Cloud environment found (see Money) - cannot mint a pairing code yet.
              </span>
            )}
          </div>
        )}

        {qr !== null && remaining > 0 && (
          <div className="flex flex-col items-center gap-2 py-2">
            <QrCode value={qr.qr_content} size={220} />
            <span className="mono text-[11px]" style={{ color: "var(--faint)" }}>
              expires in {remaining}s - scan with TokenFuse Pocket
            </span>
            <button
              type="button"
              className="icon-btn"
              style={{ width: "auto", padding: "0 12px", fontSize: 11 }}
              onClick={onCancel}
            >
              Cancel
            </button>
          </div>
        )}

        {qr !== null && remaining === 0 && (
          <div className="flex flex-col gap-2">
            <span className="mono text-[11.5px]" style={{ color: "var(--sev-medium)" }}>
              the pairing window expired unredeemed.
            </span>
            <button
              type="button"
              className="icon-btn"
              style={{ width: "auto", padding: "0 12px", fontSize: 11, alignSelf: "flex-start" }}
              onClick={() => setQr(null)}
            >
              Mint a new code
            </button>
          </div>
        )}

        {status.state === "paired" && (
          <div className="flex flex-col gap-2">
            <div className="flex flex-col gap-1 mono text-[11.5px]" style={{ color: "var(--fg)" }}>
              <span>
                {status.name || "(unnamed device)"} · {status.platform || "unknown platform"}
              </span>
              <span style={{ color: "var(--dim)" }}>device_id: {status.device_id}</span>
              <span style={{ color: "var(--dim)" }}>paired {unixToTimestamp(status.paired_at_unix)}</span>
              <span style={{ color: "var(--dim)" }}>
                last seen {unixToTimestamp(status.last_seen_unix)}
              </span>
            </div>
            <button
              type="button"
              className="icon-btn"
              style={{ width: "auto", padding: "0 14px", fontSize: 11, alignSelf: "flex-start" }}
              onClick={() => void onDisconnect()}
              disabled={disconnecting}
            >
              {disconnecting ? "Disconnecting..." : "Disconnect"}
            </button>
          </div>
        )}

        {error && (
          <div className="mono text-[11.5px] px-3 py-2" style={ERROR_BANNER_STYLE}>
            {describePocketError(error)}
          </div>
        )}
      </div>
    </div>
  );
}
