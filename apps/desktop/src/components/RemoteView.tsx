import { useCallback, useEffect, useState } from "react";
import { cssVar } from "../lib/cssVars";
import { useRemoteStatus } from "../lib/useRemoteStatus";
import type { RemoteStatus } from "../remoteTypes";
import { RemoteEnvironmentForm } from "./RemoteEnvironmentForm";
import { RemoteHetznerInventory } from "./RemoteHetznerInventory";
import { RemoteSshOps } from "./RemoteSshOps";
import { RemoteTunnelPanel } from "./RemoteTunnelPanel";

function SectionHeader({ title, detail }: { title: string; detail?: string }) {
  return (
    <div className="flex items-baseline gap-2">
      <span className="mono" style={{ fontSize: 11, letterSpacing: "0.1em", textTransform: "uppercase", color: "var(--faint)" }}>
        {title}
      </span>
      {detail && (
        <span className="text-[11px]" style={{ color: "var(--faint)" }}>
          {detail}
        </span>
      )}
    </div>
  );
}

/**
 * The Remote (Distance) panel (docs/PHASE4.md W4): manages the TRANSPORT to
 * a client-hosted stack - Hetzner inventory (read-only), the WireGuard
 * tunnel (the primary console<->Cloud channel, decision D11), and SSH ops
 * (reachability, remote descriptor read, remote log tail). Does NOT re-point
 * any other plane's Cloud connection through the tunnel - that is the LIVE
 * Hetzner exit-gate, not this local build (docs/PHASE4.md W4 "v1 SCOPE").
 *
 * Owns the ONE `status: RemoteStatus` every section reads and writes back
 * into: `useRemoteStatus` seeds it on load, and every mutating action
 * (`remote_set_environment`/`remote_wg_connect`/`remote_wg_disconnect`/
 * `remote_ssh_tail_start`/`remote_ssh_tail_stop`) already returns the fresh
 * whole-panel status (see `lib/remote.ts`), so a child calls `onStatusChange`
 * with that return value directly rather than triggering a second fetch.
 */
export function RemoteView() {
  const polled = useRemoteStatus();
  const [status, setStatus] = useState<RemoteStatus | null>(null);

  useEffect(() => {
    if (polled !== null) setStatus(polled);
  }, [polled]);

  const onStatusChange = useCallback((next: RemoteStatus) => setStatus(next), []);

  if (status === null || status.state === "bootstrapping") {
    return (
      <div className="flex-1 min-h-0 flex items-center justify-center">
        <div className="mono text-[12px]" style={{ color: "var(--faint)" }}>
          resolving the Remote panel...
        </div>
      </div>
    );
  }

  const { environment, console_public_b64: consolePublicB64, tunnel, tail, default_wireguard_go_bin: defaultBin } = status;

  return (
    <div className="flex-1 min-h-0 overflow-y-auto thin-scroll px-5 py-4 flex flex-col gap-6">
      <div className="flex flex-wrap items-center gap-2">
        <span className="chip" style={cssVar("dot", "var(--src-remote)")}>
          <span className="dot" aria-hidden="true" />
          {environment ? `environment "${environment.name}" saved` : "no environment saved yet"}
        </span>
        <span className="chip" style={cssVar("dot", "var(--faint)")}>
          <span className="dot" aria-hidden="true" />
          {defaultBin ? `wireguard-go default: ${defaultBin}` : "no wireguard-go default resolved"}
        </span>
      </div>

      <section className="flex flex-col gap-2">
        <SectionHeader title="Hetzner inventory" detail="read-only - id / name / status / ipv4 / type / cores / RAM / price-per-hour" />
        <RemoteHetznerInventory />
      </section>

      <section className="flex flex-col gap-2">
        <SectionHeader title="Remote environment" detail="the WG peer, tunnel addressing, and the SSH target for one campaign" />
        <RemoteEnvironmentForm environment={environment} defaultWireguardGoBin={defaultBin} onSaved={onStatusChange} />
      </section>

      <section className="flex flex-col gap-2">
        <SectionHeader title="WireGuard tunnel" detail="the primary console-to-Cloud channel (decision D11)" />
        <RemoteTunnelPanel
          environment={environment}
          consolePublicB64={consolePublicB64}
          tunnel={tunnel}
          onStatusChange={onStatusChange}
        />
      </section>

      <section className="flex flex-col gap-2">
        <SectionHeader title="SSH ops" detail="reachability, remote descriptor read, and a live remote log tail" />
        <RemoteSshOps hasEnvironment={environment !== null} tail={tail} onStatusChange={onStatusChange} />
      </section>
    </div>
  );
}
