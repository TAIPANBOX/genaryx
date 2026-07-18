/**
 * Pocket panel wire types (docs/PHASE5.md W2). Mirrors the Rust DTOs in
 * `src-tauri/src/pocket/commands.rs` field-for-field (same convention
 * `remoteTypes.ts`/`moneyTypes.ts` follow for their own panels).
 */

/** Mirrors `pocket::commands::PocketStatusDto`
 * (`#[serde(tag = "state", rename_all = "snake_case")]`). There is no
 * "showing_qr" state here on purpose: that step is frontend-only, entered
 * right after a successful `pocketConnect()` call and left the moment
 * `pocket_status` next reports `paired` - the relay itself exposes no "is a
 * window currently armed" read, only device-paired state. */
export type PocketStatus =
  | { state: "idle"; cloud_ready: boolean }
  | {
      state: "paired";
      device_id: string;
      name: string;
      platform: string;
      paired_at_unix: number;
      last_seen_unix: number;
    }
  | { state: "relay_unreachable"; message: string };

/** Mirrors `pocket::commands::PocketQrDto` - `pocketConnect()`'s success
 * return. `qr_content` is the EXACT `genaryx-pocket://pair/v1?...` string
 * (docs/PHASE5.md W2); render it verbatim, never reconstruct it here. */
export interface PocketQr {
  qr_content: string;
  expires_unix: number;
}

/** Mirrors `pocket::commands::PocketError`
 * (`#[serde(tag = "kind", rename_all = "snake_case")]`). */
export type PocketError =
  | { kind: "no_cloud_environment" }
  | { kind: "cloud"; message: string }
  | { kind: "device_exists" }
  | { kind: "relay"; message: string };
