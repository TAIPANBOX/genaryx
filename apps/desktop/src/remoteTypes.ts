/**
 * Remote (Distance) wire types (docs/PHASE4.md W4). Mirrors the Rust DTOs in
 * `src-tauri/src/remote/commands.rs` field-for-field (same convention
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

/** Mirrors `remote::commands::RemoteError` (`#[serde(tag = "kind", rename_all = "snake_case")]`). */
export type RemoteError =
  | { kind: "bootstrapping" }
  | { kind: "no_environment" }
  | { kind: "invalid"; message: string }
  | { kind: "no_wireguard_go_binary" }
  | { kind: "wg"; message: string }
  | { kind: "ssh"; message: string }
  | { kind: "hetzner"; message: string }
  | { kind: "internal"; message: string };

/** `remote:tail-line` Tauri event payload - mirrors `remote::commands::RemoteTailLine`. */
export interface RemoteTailLine {
  path: string;
  line: string;
}

/** `remote:tail-ended` Tauri event payload - mirrors `remote::commands::RemoteTailEnded`. */
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
