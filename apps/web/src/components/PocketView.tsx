import { useCallback, useEffect, useState } from "react";
import { cssVar } from "../lib/cssVars";
import { formatTimestamp } from "../lib/format";
import { describePocketError, pocketConnect, pocketDisconnect } from "../lib/pocket";
import { usePocketStatus } from "../lib/usePocketStatus";
import type { PocketDevice, PocketError, PocketQr, PocketStatus, PocketWindow } from "../pocketTypes";
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
      return status.cloud_ready ? "no devices paired" : "no devices paired · Cloud not resolvable";
    case "paired": {
      // At least one of the two is always set in this state (that is what
      // makes it "paired" rather than "idle" - see `pocketTypes.ts`'s own
      // doc), but a device can be disconnected on its own, so either can be
      // null here independently.
      const parts = [
        status.phone && `phone: ${status.phone.name || status.phone.device_id}`,
        status.watch && `watch: ${status.watch.name || status.watch.device_id}`,
      ].filter(Boolean);
      return `paired · ${parts.join(", ")}`;
    }
    case "relay_unreachable":
      return "relay unreachable";
  }
}

/** A small warning line for one currently armed window's probing count -
 * renders nothing at all while `failed_attempts` is 0 (the normal, quiet
 * steady state). PURELY OBSERVATIONAL: the relay never closes a window over
 * this (the pairing route is pre-auth, so it can't without letting an
 * unauthenticated caller deny pairing at will), so the copy deliberately
 * never implies blocking, lockout, or that the window will close itself -
 * it is only ever "here is what happened, use Disconnect if you want to act
 * on it". */
function PairingProbeNote({ label, pairingWindow }: { label: string; pairingWindow: PocketWindow | null }) {
  if (pairingWindow === null || pairingWindow.failed_attempts === 0) return null;
  const n = pairingWindow.failed_attempts;
  return (
    <span className="mono text-[11px]" style={{ color: "var(--sev-medium)" }}>
      {label}: {n} invalid code{n === 1 ? "" : "s"} presented since arming
    </span>
  );
}

/** Pulls `phone_window`/`watch_window` out of whichever `PocketStatus`
 * variant is current - both `"idle"` and `"paired"` carry them (see
 * `pocketTypes.ts`'s doc for why), only `"relay_unreachable"` has neither. */
function windowsOf(status: PocketStatus): { phone: PocketWindow | null; watch: PocketWindow | null } {
  if (status.state === "relay_unreachable") return { phone: null, watch: null };
  return { phone: status.phone_window, watch: status.watch_window };
}

/** One device row within the paired card - `device === null` renders the
 * slot's honest "not paired" placeholder rather than being omitted, so the
 * operator always sees both the phone and the watch slots at a glance.
 * `pairingWindow` is normally `null` once `device` is set (a successful
 * redemption closes that slot's window at the relay) - it matters for the
 * `device === null` case: the watch's window commonly outlives the phone's
 * own pairing while it waits on a WatchConnectivity handoff, so its probe
 * count needs to stay visible even after the phone row above already shows
 * paired. */
function PocketDeviceRow({
  label,
  device,
  pairingWindow,
}: {
  label: string;
  device: PocketDevice | null;
  pairingWindow: PocketWindow | null;
}) {
  if (device === null) {
    return (
      <div className="flex flex-col gap-1">
        <div className="mono text-[11.5px]" style={{ color: "var(--faint)" }}>
          {label}: not paired
        </div>
        <PairingProbeNote label={label} pairingWindow={pairingWindow} />
      </div>
    );
  }
  return (
    <div className="flex flex-col gap-1 mono text-[11.5px]" style={{ color: "var(--fg)" }}>
      <span>
        {label}: {device.name || "(unnamed device)"} · {device.platform || "unknown platform"}
      </span>
      <span style={{ color: "var(--dim)" }}>device_id: {device.device_id}</span>
      <span style={{ color: "var(--dim)" }}>paired {unixToTimestamp(device.paired_at_unix)}</span>
      <span style={{ color: "var(--dim)" }}>last seen {unixToTimestamp(device.last_seen_unix)}</span>
    </div>
  );
}

/**
 * The Pocket panel (docs/PHASE5.md W2, itrat-console/13 D12.2a): "Connect
 * TokenFuse Pocket" mints a pairing code for the phone and one for the
 * watch at the Cloud, arms both of the relay's pairing windows, and renders
 * the QR (both codes) the phone scans - a later wave (W3) builds the
 * scanner itself. Three states: idle (Connect button), showing-QR (both
 * windows armed, waiting for the phone), and paired (each slot's device
 * details, or "not paired", + Disconnect).
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
  const windows = status !== null ? windowsOf(status) : { phone: null, watch: null };

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
    // "Cancel" reuses Disconnect (all slots): nothing is actually paired
    // yet in the common case at this point in the flow, so this normally
    // only closes the just-armed windows (`RelayAdminClient::disconnect`
    // clears a pairing-window row regardless of whether a device is paired,
    // `registry.rs::disconnect`) - an operator-initiated version of
    // docs/PHASE5.md W2's "do not leave a half-armed window silently" rule,
    // not just the error-path cleanup `pocket_connect` itself already does
    // server-side. If one slot (say the watch, over WatchConnectivity)
    // happened to pair before the operator cancelled, "all slots" is still
    // the right scope: it leaves neither a half-paired relay nor an orphaned
    // single device the panel has no view of.
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
          Pair your phone (TokenFuse Pocket) and its paired Watch to this box&apos;s relay so you can see
          the exception queue and slide-to-kill a runaway from anywhere - one QR carries the relay&apos;s
          pinned TLS identity plus a one-time code for each device, scanned once on the phone (which hands
          the Watch its own code), no manual entry.
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
            <PairingProbeNote label="phone" pairingWindow={windows.phone} />
            <PairingProbeNote label="watch" pairingWindow={windows.watch} />
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
          <div className="flex flex-col gap-3">
            <div className="flex flex-col gap-2">
              <PocketDeviceRow label="Phone" device={status.phone} pairingWindow={status.phone_window} />
              <PocketDeviceRow label="Watch" device={status.watch} pairingWindow={status.watch_window} />
            </div>
            {/* Disconnect always frees BOTH slots at once (there is no
                per-slot disconnect affordance): the two devices are paired
                together by one Connect flow, so resetting either one to pair
                again also resets the other, rather than leaving a stray
                paired device the operator cannot see a fresh QR for. */}
            <button
              type="button"
              className="icon-btn"
              style={{ width: "auto", padding: "0 14px", fontSize: 11, alignSelf: "flex-start" }}
              onClick={() => void onDisconnect()}
              disabled={disconnecting}
            >
              {disconnecting ? "Disconnecting..." : "Disconnect all"}
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
