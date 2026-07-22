import { hasBackend, invokeBackend } from "./transport";
import type {
  CloudListOptions,
  CloudProviderId,
  CloudServer,
  HetznerServer,
  RemoteEnvironmentRequest,
  RemoteError,
  RemoteFile,
  RemoteStatus,
} from "../remoteTypes";

/** Thrown by every mutator below when there is no Tauri runtime to talk to -
 * mirrors `lib/evidence.ts`'s identical `NO_TAURI_ERROR` guard. */
const NO_TAURI_ERROR: RemoteError = { kind: "internal", message: "no Tauri runtime available" };

/** The honest, SETTLED fallback shape for `fetchRemoteStatus` outside Tauri
 * (or on a genuine IPC failure) - mirrors `lib/evidence.ts`'s
 * `EVIDENCE_UNAVAILABLE`: NOT `{state:"bootstrapping"}`, which would leave
 * the panel showing "resolving..." forever in a plain browser preview
 * instead of settling like a real, freshly-booted, unconfigured panel would
 * (`RemoteStatusDto` has no separate `no_environment` variant to fall back
 * to instead - see `remote::state`'s module doc for why). */
const REMOTE_UNAVAILABLE: RemoteStatus = {
  state: "ready",
  default_wireguard_go_bin: null,
  environment: null,
  console_public_b64: null,
  tunnel: { state: "disconnected" },
  tail: null,
};

/** Normalize whatever `invoke()` rejected with into a `RemoteError`. Tauri
 * passes a command's `Err` value through as the structured object it was
 * serialized from, so this is normally already a `RemoteError` in disguise;
 * the fallback branch only matters for a transport-level IPC failure. */
function toRemoteError(err: unknown): RemoteError {
  if (err && typeof err === "object" && "kind" in err) {
    return err as RemoteError;
  }
  return { kind: "internal", message: err instanceof Error ? err.message : String(err) };
}

async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (!hasBackend()) throw NO_TAURI_ERROR;
  try {
    return await invokeBackend<T>(command, args);
  } catch (err) {
    throw toRemoteError(err);
  }
}

/** Whole-panel state. Never throws: outside Tauri (or on any IPC failure) it
 * settles to [`REMOTE_UNAVAILABLE`] instead of hanging on `bootstrapping` -
 * mirrors `lib/evidence.ts`'s `fetchEvidenceStatus`. `remote_status` itself
 * never fails on the Rust side either, so the catch branch only matters for
 * a genuine transport failure. */
export async function fetchRemoteStatus(): Promise<RemoteStatus> {
  if (!hasBackend()) return REMOTE_UNAVAILABLE;
  try {
    return await invokeBackend<RemoteStatus>("remote_status");
  } catch {
    return REMOTE_UNAVAILABLE;
  }
}

/** Save (or replace) the operator-defined remote environment - resets any
 * live tunnel/SSH client/tail, see `remote::commands::remote_set_environment`'s
 * doc comment. Returns the fresh whole-panel status (no second round trip
 * needed). */
export const setRemoteEnvironment = (request: RemoteEnvironmentRequest): Promise<RemoteStatus> =>
  call<RemoteStatus>("remote_set_environment", { request });

/** Hetzner inventory (STRICTLY READ-ONLY - `HetznerClient` exposes no
 * create/delete/mutate method at all, so nothing in this app can touch
 * Hetzner infrastructure beyond listing it). Stateless: the token is never
 * persisted server-side, only used for this one call.
 * `labelSelector` blank -> the connector's own `managed-by=taipan` default. */
export const listHetznerServers = (token: string, labelSelector: string): Promise<HetznerServer[]> =>
  call<HetznerServer[]>("remote_hetzner_list", {
    token,
    label_selector: labelSelector.trim().length > 0 ? labelSelector : null,
  });

/** Cloud VM inventory (STRICTLY READ-ONLY) for a provider with a built-in
 * connector (`aws`/`gcp`/`azure`), via the operator's OWN already-authenticated
 * official CLI. The console stores none of the provider's credentials and the
 * connector only ever runs the describe/list command - it can never create,
 * resize, or delete a resource. */
export const listCloudServers = (provider: CloudProviderId, options: CloudListOptions): Promise<CloudServer[]> =>
  call<CloudServer[]>("remote_cloud_list", { provider, options });

/** Bring the WireGuard tunnel up: generates the console's WG identity on
 * first use, then attempts `WgTunnel::bring_up`. Fail-closed: a failed
 * bring-up still RESOLVES (never rejects) with `tunnel.state === "failed"` -
 * only a genuine IPC/task failure throws. Locally (no privileged helper)
 * this is expected to fail with a privilege error - see `RemoteTunnelPanel`. */
export const connectTunnel = (): Promise<RemoteStatus> => call<RemoteStatus>("remote_wg_connect");

/** Tear the tunnel down (drop it) - always safe, even when already
 * disconnected or already failed. */
export const disconnectTunnel = (): Promise<RemoteStatus> => call<RemoteStatus>("remote_wg_disconnect");

/** A reachability + host-key-pin + auth probe. */
export const checkSshReachable = (): Promise<void> => call<void>("remote_ssh_check_reachable");

/** Read one remote file's bytes (e.g. a taipan descriptor). */
export const readRemoteFile = (path: string): Promise<RemoteFile> =>
  call<RemoteFile>("remote_ssh_read_file", { path });

/** Start (replacing any previous) a streaming remote tail - lines arrive
 * over the `remote:tail-line` Tauri event (see `RemoteSshOps.tsx`). */
export const startRemoteTail = (path: string, fromOffset: number): Promise<RemoteStatus> =>
  call<RemoteStatus>("remote_ssh_tail_start", { path, from_offset: fromOffset });

/** Stop the in-flight remote tail, if any (always safe to call). */
export const stopRemoteTail = (): Promise<RemoteStatus> => call<RemoteStatus>("remote_ssh_tail_stop");

/** Human-readable text for any `RemoteError` - used for the plain error
 * banner, mirrors every sibling panel's `describe*Error`. */
export function describeRemoteError(err: RemoteError): string {
  switch (err.kind) {
    case "bootstrapping":
      return "Still resolving the Remote panel.";
    case "no_environment":
      return "No remote environment saved yet - fill in the environment form first.";
    case "invalid":
      return err.message;
    case "no_wireguard_go_binary":
      return "No wireguard-go binary resolved - set its path in the environment form.";
    case "wg":
      return err.message;
    case "ssh":
      return err.message;
    case "hetzner":
      return err.message;
    case "cloud":
      return err.message;
    case "internal":
      return err.message;
  }
}
