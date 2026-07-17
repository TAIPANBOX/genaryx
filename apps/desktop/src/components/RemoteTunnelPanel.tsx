import { useCallback, useState } from "react";
import { cssVar } from "../lib/cssVars";
import { connectTunnel, describeRemoteError, disconnectTunnel } from "../lib/remote";
import type { RemoteEnvironment, RemoteError, RemoteStatus, TunnelStatus } from "../remoteTypes";

function tunnelDotColor(tunnel: TunnelStatus): string {
  switch (tunnel.state) {
    case "connected":
      return "var(--sev-low)";
    case "connecting":
      return "var(--sev-medium)";
    case "failed":
      return "var(--sev-high)";
    case "disconnected":
      return "var(--faint)";
  }
}

function tunnelLabel(tunnel: TunnelStatus): string {
  switch (tunnel.state) {
    case "disconnected":
      return "disconnected";
    case "connecting":
      return "connecting...";
    case "connected": {
      const hs =
        tunnel.latest_handshake_secs !== null
          ? `last handshake ${tunnel.latest_handshake_secs}s (epoch)`
          : "no handshake yet";
      return `connected · ${tunnel.interface} · ${hs}`;
    }
    case "failed":
      return `FAILED: ${tunnel.message}`;
  }
}

/**
 * Section 3 (docs/PHASE4.md W4 position 3): Connect/Disconnect the WireGuard
 * tunnel, the console's own WG public key (to hand the box admin), and an
 * honest tunnel-status readout. Fail-closed by construction: `Connect`
 * NEVER shows "connected" unless `tunnel.state === "connected"` came back
 * from a real `WgTunnel::bring_up` success - a failed bring-up renders as a
 * loud FAILED banner with the exact `WgError` text, never silently hidden.
 */
export function RemoteTunnelPanel({
  environment,
  consolePublicB64,
  tunnel,
  onStatusChange,
}: {
  environment: RemoteEnvironment | null;
  consolePublicB64: string | null;
  tunnel: TunnelStatus;
  onStatusChange: (status: RemoteStatus) => void;
}) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<RemoteError | null>(null);
  const [copied, setCopied] = useState(false);

  const onConnect = useCallback(async () => {
    if (busy || environment === null) return;
    setBusy(true);
    setError(null);
    try {
      const status = await connectTunnel();
      onStatusChange(status);
    } catch (err) {
      setError(err as RemoteError);
    } finally {
      setBusy(false);
    }
  }, [busy, environment, onStatusChange]);

  const onDisconnect = useCallback(async () => {
    if (busy) return;
    setBusy(true);
    setError(null);
    try {
      const status = await disconnectTunnel();
      onStatusChange(status);
    } catch (err) {
      setError(err as RemoteError);
    } finally {
      setBusy(false);
    }
  }, [busy, onStatusChange]);

  const onCopyKey = useCallback(() => {
    if (!consolePublicB64) return;
    void navigator.clipboard.writeText(consolePublicB64).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 2_000);
    });
  }, [consolePublicB64]);

  const connected = tunnel.state === "connected";

  return (
    <div className="panel px-4 py-3 flex flex-col gap-2.5" style={{ background: "var(--panel-2)" }}>
      <div className="flex items-center gap-2 flex-wrap">
        <span className="chip" style={cssVar("dot", tunnelDotColor(tunnel))}>
          <span className="dot" aria-hidden="true" />
          {tunnelLabel(tunnel)}
        </span>
      </div>

      <div className="flex flex-col gap-1">
        <span className="text-[11px]" style={{ color: "var(--dim)" }}>
          console WG public key (hand this to the box admin so they can add it as an allowed peer)
        </span>
        <div className="flex items-center gap-2">
          <code
            className="mono flex-1 truncate"
            style={{
              background: "var(--panel)",
              border: "1px solid var(--line-2)",
              borderRadius: 8,
              padding: "6px 10px",
              fontSize: 11.5,
              color: consolePublicB64 ? "var(--fg)" : "var(--faint)",
            }}
          >
            {consolePublicB64 ?? "not generated yet - click Connect to generate one"}
          </code>
          <button
            type="button"
            className="icon-btn"
            style={{ width: "auto", padding: "0 10px", fontSize: 11 }}
            onClick={onCopyKey}
            disabled={!consolePublicB64}
          >
            {copied ? "Copied" : "Copy"}
          </button>
        </div>
      </div>

      <div
        className="mono text-[11px]"
        style={{ color: "var(--faint)", lineHeight: 1.6, background: "var(--panel)", borderRadius: 8, padding: "8px 10px", border: "1px solid var(--line)" }}
      >
        wireguard-go needs elevated privileges to create a network tun device. Connecting as a plain operator on
        this machine is expected to fail with a privilege error, shown honestly as FAILED below - that is correct
        fail-closed behavior, not a bug. The live tunnel is validated on the Hetzner campaign box (which runs a
        privileged helper), not on a local dev machine.
      </div>

      <div className="flex items-center gap-3 flex-wrap">
        <button
          type="button"
          className="icon-btn"
          style={{ width: "auto", padding: "0 14px", fontSize: 11 }}
          onClick={() => void onConnect()}
          disabled={busy || environment === null || connected}
        >
          {busy && !connected ? "Connecting..." : "Connect"}
        </button>
        <button
          type="button"
          className="icon-btn"
          style={{ width: "auto", padding: "0 14px", fontSize: 11 }}
          onClick={() => void onDisconnect()}
          disabled={busy || tunnel.state === "disconnected"}
        >
          Disconnect
        </button>
        {environment === null && (
          <span className="text-[11px]" style={{ color: "var(--faint)" }}>
            save an environment above first.
          </span>
        )}
      </div>

      {error && (
        <div className="panel px-3 py-2 mono text-[11.5px]" style={{ background: "var(--panel)", color: "var(--sev-high)" }}>
          {describeRemoteError(error)}
        </div>
      )}
    </div>
  );
}
