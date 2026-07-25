import { useCallback, useEffect, useState } from "react";
import { cssVar } from "../lib/cssVars";
import { PopoverHeader } from "../lib/popover";
import {
  describeRemoteError,
  downloadWgOperatorConfig,
  issueOperatorWgConfig,
  listOperatorWgPeers,
  revokeOperatorWgPeer,
} from "../lib/remote";
import { useSession } from "../lib/useSession";
import type { RemoteError, RemoteWgOperatorConfig, RemoteWgPeer } from "../remoteTypes";

/** A filesystem-safe slug for the downloaded `.conf`'s filename, from the
 * signed-in operator's own session username - falls back to "operator" when
 * there is no session yet (the mock preview has none, `lib/useSession.ts`'s
 * own doc comment) or the username has no safe characters at all. */
function safeFileSlug(name: string | null | undefined): string {
  const slug = (name ?? "")
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9_-]+/g, "-")
    .replace(/^-+|-+$/g, "");
  return slug.length > 0 ? slug : "operator";
}

/** How long ago this device last completed a handshake, or the honest "never".
 *
 * A peer that was issued and never used looks exactly like one that was issued
 * to the wrong person, so the two are never collapsed into the same wording. */
function describeHandshake(unix: number | null): string {
  if (unix === null) return "never connected";
  const seconds = Math.max(0, Math.floor(Date.now() / 1000) - unix);
  if (seconds < 90) return "connected just now";
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `last seen ${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 48) return `last seen ${hours}h ago`;
  return `last seen ${Math.floor(hours / 24)}d ago`;
}

/** Bytes as something readable at a glance; exact counts matter to nobody
 * looking at a device list. */
function describeBytes(rx: number, tx: number): string {
  const total = rx + tx;
  if (total === 0) return "no traffic";
  const units = ["B", "KB", "MB", "GB"];
  let n = total;
  let u = 0;
  while (n >= 1024 && u < units.length - 1) {
    n /= 1024;
    u += 1;
  }
  return `${n < 10 ? n.toFixed(1) : Math.round(n)} ${units[u]}`;
}

/**
 * Every device currently holding a way into this box, with a revoke beside
 * each one.
 *
 * This exists because issuing without revoking is not a feature, it is a leak:
 * the first `.conf` handed out would otherwise be permanent access to the
 * control plane, with no way to see it and no way to take it back.
 *
 * Revoking is deliberately two clicks. The passkey ceremony already confirms a
 * human is present, but it cannot know that the row under the cursor is the
 * operator's OWN device: a single misclick would end their session's way in
 * and leave them outside a console that is only reachable through it.
 */
function WgPeerList({ justIssued }: { justIssued: string | null }) {
  const session = useSession();
  const [peers, setPeers] = useState<RemoteWgPeer[] | null>(null);
  const [error, setError] = useState<RemoteError | null>(null);
  const [confirming, setConfirming] = useState<string | null>(null);
  const [revoking, setRevoking] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      const result = await listOperatorWgPeers();
      setPeers(result.peers);
      setError(null);
    } catch (err) {
      setError(err as RemoteError);
    }
  }, []);

  // Re-read whenever a device is issued, so the row for the config still on
  // screen is actually there rather than appearing only after a manual reload.
  useEffect(() => {
    void refresh();
  }, [refresh, justIssued]);

  const onRevoke = useCallback(
    async (publicKey: string) => {
      if (confirming !== publicKey) {
        setConfirming(publicKey);
        return;
      }
      setConfirming(null);
      setRevoking(publicKey);
      setError(null);
      try {
        await revokeOperatorWgPeer(publicKey);
        await refresh();
      } catch (err) {
        setError(err as RemoteError);
      } finally {
        setRevoking(null);
      }
    },
    [confirming, refresh],
  );

  if (error && peers === null) {
    return (
      <div className="panel px-3 py-2 mono text-[11.5px]" style={{ background: "var(--panel)", color: "var(--dim)" }}>
        {describeRemoteError(error, session?.role)}
      </div>
    );
  }
  if (peers === null) return null;

  return (
    <div className="flex flex-col gap-1.5 pt-1">
      <div className="flex items-center gap-2">
        <span className="text-[10px] uppercase tracking-wider" style={{ color: "var(--faint)" }}>
          devices with a way in
        </span>
        <span className="text-[10px]" style={{ color: "var(--faint)" }}>
          {peers.length === 0 ? "none" : peers.length}
        </span>
      </div>

      {error && (
        <div className="mono text-[10.5px]" style={{ color: "var(--sev-high)" }}>
          {describeRemoteError(error, session?.role)}
        </div>
      )}

      {peers.length === 0 ? (
        <span className="text-[11px]" style={{ color: "var(--dim)" }}>
          Nothing can reach this box over the tunnel yet.
        </span>
      ) : (
        peers.map((peer) => {
          const isJustIssued = justIssued !== null && peer.public_key === justIssued;
          const isConfirming = confirming === peer.public_key;
          const isRevoking = revoking === peer.public_key;
          return (
            <div
              key={peer.public_key}
              className="panel px-3 py-2 flex items-center gap-3 flex-wrap"
              style={{ background: "var(--panel)" }}
            >
              <code className="mono text-[11.5px]" style={{ color: "var(--fg)", minWidth: 92 }}>
                {peer.allowed_ips[0] ?? "no address"}
              </code>
              <span className="text-[10.5px]" style={{ color: "var(--dim)", minWidth: 130 }}>
                {describeHandshake(peer.last_handshake_unix)}
              </span>
              <span className="mono text-[10.5px]" style={{ color: "var(--faint)", minWidth: 70 }}>
                {describeBytes(peer.rx_bytes, peer.tx_bytes)}
              </span>
              {isJustIssued && (
                <span className="chip" style={cssVar("dot", "var(--sev-low)")}>
                  <span className="dot" aria-hidden="true" />
                  just issued
                </span>
              )}
              {/* The key itself, truncated: it is the only thing that
                  distinguishes two devices on the same address after a
                  re-issue, so it has to be visible somewhere. */}
              <code className="mono text-[10px]" style={{ color: "var(--faint)" }} title={peer.public_key}>
                {peer.public_key.slice(0, 12)}...
              </code>
              <button
                type="button"
                className="icon-btn"
                style={{
                  width: "auto",
                  padding: "0 10px",
                  fontSize: 10.5,
                  marginLeft: "auto",
                  color: isConfirming ? "var(--sev-high)" : undefined,
                }}
                onClick={() => void onRevoke(peer.public_key)}
                disabled={isRevoking}
                title={
                  isConfirming
                    ? "This cuts the device off immediately. Click again to confirm."
                    : "Revoke this device's access to the tunnel"
                }
              >
                {isRevoking ? "Revoking..." : isConfirming ? "Confirm revoke" : "Revoke"}
              </button>
            </div>
          );
        })
      )}
    </div>
  );
}

/**
 * "Connect this machine": issue the signed-in operator a fresh WireGuard
 * peer against THIS box's own kernel WireGuard server, so their laptop or
 * phone can reach the console over the tunnel instead of SSH - the OPPOSITE
 * direction from `RemoteTunnelPanel`'s Connect/Disconnect, which dials the
 * console itself OUT to a remote box (see `genaryx_api::remote::wg_operator`'s
 * module doc for the full rationale).
 *
 * Content-only, no title of its own, so it drops unchanged into `RemoteView`'s
 * own `SectionHeader` (the must-have placement) and into a popover window's
 * `PopoverHeader` via [`RemoteWgOperatorPopoverCard`] below (the post-login
 * nice-to-have, opened from `AppHeader`'s session area, matching how
 * `PasskeySettings` opens as a popover window rather than living inline in
 * the rail).
 *
 * Side-effect-honest: "Issue WireGuard config" really adds a peer to the box's
 * live interface, it is not a preview - matches
 * `genaryx_api::remote::wg_operator::operator_wg_config`'s own contract.
 * Never renders the client's private key as plain page text: it only ever
 * leaves this component inside the QR image and the downloaded `.conf`,
 * both of which the operator explicitly asked for.
 */
export function RemoteWgOperatorCard() {
  const session = useSession();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<RemoteError | null>(null);
  const [result, setResult] = useState<RemoteWgOperatorConfig | null>(null);

  const onIssue = useCallback(async () => {
    if (busy) return;
    setBusy(true);
    setError(null);
    try {
      const config = await issueOperatorWgConfig();
      setResult(config);
    } catch (err) {
      setError(err as RemoteError);
    } finally {
      setBusy(false);
    }
  }, [busy]);

  const onDownload = useCallback(() => {
    if (!result) return;
    downloadWgOperatorConfig(result, `genaryx-${safeFileSlug(session?.user)}.conf`);
  }, [result, session?.user]);

  return (
    <div className="panel px-4 py-3 flex flex-col gap-2.5" style={{ background: "var(--panel-2)" }}>
      <span className="text-[11.5px]" style={{ color: "var(--dim)", lineHeight: 1.5, maxWidth: 640 }}>
        This connects your own laptop or phone to this box over WireGuard, so you can reach the console through the
        tunnel instead of SSH. Import the config into the official WireGuard app, or scan the QR, then open the
        console at the tunnel URL below.
      </span>

      <div className="flex items-center gap-3 flex-wrap">
        <button
          type="button"
          className="icon-btn"
          style={{ width: "auto", padding: "0 14px", fontSize: 11 }}
          onClick={() => void onIssue()}
          disabled={busy}
        >
          {busy ? "Issuing..." : "Issue WireGuard config"}
        </button>
        {result && (
          <span className="chip" style={cssVar("dot", "var(--sev-low)")}>
            <span className="dot" aria-hidden="true" />
            issued at {result.client_ip}
          </span>
        )}
      </div>

      {error && (
        <div
          className="panel px-3 py-2 mono text-[11.5px]"
          style={{ background: "var(--panel)", color: "var(--sev-high)" }}
        >
          {describeRemoteError(error, session?.role)}
        </div>
      )}

      {result && (
        <div className="flex items-start gap-4 flex-wrap pt-1">
          {result.qr_svg ? (
            // The SVG is generated by our own Rust QR encoder from the config
            // text on this same response, never from anything user-supplied,
            // so there is no untrusted markup to sanitise here.
            <div
              role="img"
              aria-label="WireGuard config QR code"
              style={{
                width: 220,
                height: 220,
                borderRadius: 8,
                border: "1px solid var(--line-2)",
                background: "#fff",
                padding: 6,
                // The SVG carries its own intrinsic size, which is larger than
                // this box, so without this it renders at full size and paints
                // over the details beside it rather than scaling down.
                display: "grid",
                placeItems: "center",
                overflow: "hidden",
              }}
              // eslint-disable-next-line react/no-danger
              ref={(el) => {
                const svg = el?.querySelector("svg");
                if (svg) {
                  svg.setAttribute("width", "100%");
                  svg.setAttribute("height", "100%");
                  svg.style.maxWidth = "100%";
                  svg.style.maxHeight = "100%";
                }
              }}
              dangerouslySetInnerHTML={{ __html: result.qr_svg }}
            />
          ) : (
            <div
              className="flex items-center justify-center mono text-[10.5px] text-center"
              style={{
                width: 220,
                height: 220,
                borderRadius: 8,
                border: "1px dashed var(--line-2)",
                color: "var(--faint)",
                padding: 12,
              }}
            >
              no QR available in this preview
            </div>
          )}
          <div className="flex flex-col gap-2.5" style={{ minWidth: 220 }}>
            <button
              type="button"
              className="icon-btn"
              style={{ width: "auto", padding: "0 14px", fontSize: 11, alignSelf: "flex-start" }}
              onClick={onDownload}
            >
              Download WireGuard config
            </button>
            <div className="flex flex-col gap-1">
              <span className="text-[10px] uppercase tracking-wider" style={{ color: "var(--faint)" }}>
                tunnel address
              </span>
              <code className="mono text-[11.5px]" style={{ color: "var(--fg)" }}>
                {result.client_ip}
              </code>
            </div>
            <div className="flex flex-col gap-1">
              <span className="text-[10px] uppercase tracking-wider" style={{ color: "var(--faint)" }}>
                console over the tunnel, once connected
              </span>
              <code className="mono text-[11.5px]" style={{ color: "var(--fg)" }}>
                {result.console_tunnel_url}
              </code>
            </div>
          </div>
        </div>
      )}

      <WgPeerList justIssued={result?.peer_public_key ?? null} />
    </div>
  );
}

/**
 * [`RemoteWgOperatorCard`], wrapped with its own `PopoverHeader` - opened
 * from `AppHeader`'s session area (`usePopover`), matching how
 * `PasskeySettings` opens as a popover window rather than living inline in
 * the rail (the post-login nice-to-have placement; the Remote-view card
 * above is the must-have one).
 */
export function RemoteWgOperatorPopoverCard() {
  return (
    <div className="flex flex-col">
      <PopoverHeader kicker="Session" title="Connect this machine" />
      <div style={{ padding: "0 16px 16px" }}>
        <RemoteWgOperatorCard />
      </div>
    </div>
  );
}
