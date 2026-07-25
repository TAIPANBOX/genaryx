/**
 * Remote (Distance) wire types (docs/PHASE4.md W4). Mirrors the Rust DTOs in
 * `crates/api/src/remote/commands.rs` field-for-field (same convention
 * `memoryTypes.ts`/`evidenceTypes.ts` follow for their own panels).
 *
 * `HetznerServer` mirrors `genaryx_connectors::HetznerServer`
 * (`crates/connectors/src/hetzner.rs`) directly - it already derives
 * `Serialize` with no `rename_all`, so its wire shape is plain snake_case
 * field names, same as every other connector-owned DTO in this app.
 */

/** Mirrors `remote::commands::RemoteStatusDto`
 * (`#[serde(tag = "state", rename_all = "snake_case")]`). No
 * `no_environment` variant: Hetzner/WG/SSH are three independent
 * capabilities, not one all-or-nothing plane - `environment`/`tunnel`/`tail`
 * each report their own readiness independently once `state === "ready"`. */
export type RemoteStatus =
  | { state: "bootstrapping" }
  | {
      state: "ready";
      default_wireguard_go_bin: string | null;
      environment: RemoteEnvironment | null;
      /** The console's own WG public key, once generated - `null` until the
       * first Connect attempt. Survives an environment edit (the console's
       * identity is independent of which box it is dialing). */
      console_public_b64: string | null;
      tunnel: TunnelStatus;
      tail: TailStatus | null;
    };

/** Mirrors `remote::commands::RemoteEnvironmentDto`. */
export interface RemoteEnvironment {
  name: string;
  wireguard_go_bin: string;
  wg_peer_public_key_hex: string;
  wg_endpoint: string;
  wg_allowed_ips: string[];
  wg_persistent_keepalive: number | null;
  wg_listen_port: number | null;
  wg_local_ip: string;
  wg_peer_ip: string;
  ssh_host: string;
  ssh_port: number;
  ssh_user: string;
  ssh_identity_file: string;
  ssh_pinned_host_key: string;
}

/** What `remote_set_environment` sends - mirrors
 * `remote::commands::RemoteEnvironmentRequest` (same field shape as
 * `RemoteEnvironment`, kept as a distinct alias so a call site's intent -
 * "saving a request" vs. "rendering a resolved environment" - reads clearly). */
export type RemoteEnvironmentRequest = RemoteEnvironment;

/** Mirrors `remote::commands::TunnelStatusDto`
 * (`#[serde(tag = "state", rename_all = "snake_case")]`). `failed` is a
 * DURABLE record of the last bring-up's own `WgError` message - it persists
 * in `remote_status` until the next Connect/Disconnect, never silently
 * reverts to `disconnected` (docs/PHASE4.md W4: never claim/imply a tunnel
 * recovered on its own). */
export type TunnelStatus =
  | { state: "disconnected" }
  | { state: "connecting" }
  | { state: "connected"; interface: string; latest_handshake_secs: number | null }
  | { state: "failed"; message: string };

/** Mirrors `remote::commands::TailStatusDto`. */
export interface TailStatus {
  path: string;
  running: boolean;
}

/** Mirrors `remote::commands::RemoteFileDto` - `remote_ssh_read_file`'s
 * return. `valid_utf8: false` means `content` is a LOSSY decode (replacement
 * characters may appear in place of invalid bytes) - render that honestly,
 * never pretend it is exact. */
export interface RemoteFile {
  content: string;
  valid_utf8: boolean;
  size_bytes: number;
}

/** Mirrors `genaryx_connectors::HetznerServer` (`crates/connectors/src/hetzner.rs`).
 * READ-ONLY inventory row - there is no create/delete anywhere in this app,
 * by construction (`HetznerClient` exposes no mutation method at all). */
export interface HetznerServer {
  id: number;
  name: string;
  status: string;
  ipv4: string | null;
  server_type: string;
  cores: number;
  memory_gb: number;
  location: string;
  price_hourly_eur: number | null;
  labels: Record<string, string>;
  created: string;
}

/** The provider ids that have a built-in read-only cloud inventory connector
 * (mirrors `genaryx_connectors::CloudProvider`). */
export type CloudProviderId = "aws" | "gcp" | "azure" | "ibmcloud";

/** Mirrors `genaryx_connectors::CloudServer` - one read-only VM inventory row
 * from a provider's own official CLI (`aws`/`gcloud`/`az`/`ibmcloud`). Flat
 * snake_case, same convention as `HetznerServer`. `public_ip`/`private_ip` are
 * `null` when the instance has none. */
export interface CloudServer {
  provider: string;
  id: string;
  name: string;
  status: string;
  public_ip: string | null;
  private_ip: string | null;
  server_type: string;
  region: string;
}

/** Mirrors `genaryx_connectors::CloudListOptions` - all-optional provider
 * scoping for a cloud inventory list (region for AWS and IBM, project for
 * GCP, subscription for Azure, profile for AWS, resource_group for IBM). */
export interface CloudListOptions {
  region?: string;
  project?: string;
  subscription?: string;
  profile?: string;
  resource_group?: string;
}

/** Mirrors `remote::commands::RemoteError` (`#[serde(tag = "kind", rename_all = "snake_case")]`).
 *
 * `role_required` is the one variant NOT mirrored from the Rust command's own
 * `RemoteError` enum: it is `lib/remote.ts`'s `toRemoteError` recognizing
 * `genaryx-web`'s command-chokepoint role gate (docs/CONSOLE-IDP.md), a 403
 * that happens BEFORE the command ever reaches `remote::commands` - added here
 * client-side so the existing error banner can render it honestly. Every
 * admin-gated `remote_*` command (set_environment, wg_connect/disconnect,
 * ssh_read_file, ssh_tail_start/stop, operator_wg_config) shares that one 403
 * shape, so a viewer/approver hitting any of them lands here instead of the
 * `internal` fallback's `String(err)` (which rendered a raw object as the
 * literal "[object Object]"). */
export type RemoteError =
  | { kind: "bootstrapping" }
  | { kind: "no_environment" }
  | { kind: "invalid"; message: string }
  | { kind: "no_wireguard_go_binary" }
  | { kind: "wg_server_not_configured"; iface: string; message: string }
  | { kind: "wg"; message: string }
  | { kind: "ssh"; message: string }
  | { kind: "hetzner"; message: string }
  | { kind: "cloud"; message: string }
  | { kind: "internal"; message: string }
  | { kind: "role_required"; role: "viewer" | "approver" | "admin" };

/** Mirrors `remote::wg_operator::RemoteWgOperatorConfigDto` -
 * `remote_operator_wg_config`'s return. Everything the "Connect this
 * machine" card needs to render the QR code and offer the `.conf` download
 * in one round trip. */
export interface RemoteWgOperatorConfig {
  /** The complete client `.conf` TEXT, private key included - what the
   * Download button saves verbatim and what the QR code encodes. */
  conf: string;
  /** The QR as an inline SVG document (the `<svg>` element itself, no XML
   * prolog). SVG rather than a PNG because the console image ships no image
   * encoder and no `qrencode`: this is rendered in Rust and inlined here. */
  qr_svg: string;
  client_ip: string;
  /** `host:port` the client dials. */
  endpoint: string;
  server_public_key: string;
  /** The issued peer's public key, base64. The handle a later revoke names. */
  peer_public_key: string;
  /** Where the console answers once the tunnel is up. */
  console_tunnel_url: string;
}

/** Mirrors `remote::wg_operator::RemoteWgPeerDto`: one device currently
 * authorized on the tunnel. */
export interface RemoteWgPeer {
  public_key: string;
  allowed_ips: string[];
  /** Unix seconds of the last completed handshake, `null` if this device has
   * never connected: issued-and-never-used looks exactly like issued to the
   * wrong person, so the UI must be able to tell them apart. */
  last_handshake_unix: number | null;
  endpoint: string | null;
  rx_bytes: number;
  tx_bytes: number;
}

/** Mirrors `remote::wg_operator::RemoteWgPeersDto`. */
export interface RemoteWgPeers {
  iface: string;
  server_public_key: string | null;
  listen_port: number | null;
  /** Which backend answered: `uapi` (the sidecar) or `wg` (a kernel interface
   * on this host). Shown rather than hidden, so "it works here" is never
   * mistaken for one uniform mechanism. */
  backend: string;
  peers: RemoteWgPeer[];
}

/** Mirrors `remote::wg_operator::RemoteWgRevokeDto`. */
export interface RemoteWgRevoke {
  public_key: string;
  was_present: boolean;
  remaining_peers: number;
}

/** `remote:tail-line` SSE event payload - mirrors `remote::commands::RemoteTailLine`. */
export interface RemoteTailLine {
  path: string;
  line: string;
}

/** `remote:tail-ended` SSE event payload - mirrors `remote::commands::RemoteTailEnded`. */
export interface RemoteTailEnded {
  path: string;
  reason: string;
}

/** A blank starting point for the environment form - `wireguard_go_bin` is
 * left for the caller to pre-fill from `RemoteStatus.default_wireguard_go_bin`
 * once known. */
export function blankRemoteEnvironment(): RemoteEnvironment {
  return {
    name: "",
    wireguard_go_bin: "",
    wg_peer_public_key_hex: "",
    wg_endpoint: "",
    wg_allowed_ips: [],
    wg_persistent_keepalive: 25,
    wg_listen_port: null,
    wg_local_ip: "",
    wg_peer_ip: "",
    ssh_host: "",
    ssh_port: 22,
    ssh_user: "root",
    ssh_identity_file: "",
    ssh_pinned_host_key: "",
  };
}
