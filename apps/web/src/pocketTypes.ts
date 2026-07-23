/**
 * Pocket panel wire types (docs/PHASE5.md W2). Mirrors the Rust DTOs in
 * `crates/api/src/pocket/commands.rs` field-for-field (same convention
 * `remoteTypes.ts`/`moneyTypes.ts` follow for their own panels).
 */

/** Mirrors `pocket::commands::PocketDeviceDto` - one paired device slot,
 * shown within `PocketStatus`'s `"paired"` state. Only ever present for a
 * slot that IS paired; an empty slot is `null`/absent on the parent, not an
 * all-fields-empty record. */
export interface PocketDevice {
  device_id: string;
  name: string;
  platform: string;
  paired_at_unix: number;
  last_seen_unix: number;
}

/** Mirrors `pocket::commands::PocketWindowDto` - one currently armed pairing
 * window. `failed_attempts` (wrong codes presented to `POST /relay/v1/pair`
 * since this window was armed) is PURELY OBSERVATIONAL: the relay never
 * closes a window over it (the pairing route is pre-auth, so closing on
 * this would let an unauthenticated caller deny pairing at will). Render it
 * as something for the operator to notice and act on through the existing
 * Disconnect affordance if they choose to - never as a threat this app is
 * already handling, blocking, or that will close the window on its own. */
export interface PocketWindow {
  expires_unix: number;
  failed_attempts: number;
}

/** Mirrors `pocket::commands::PocketStatusDto`
 * (`#[serde(tag = "state", rename_all = "snake_case")]`). There is no
 * "showing_qr" state here on purpose: that step is frontend-only, entered
 * right after a successful `pocketConnect()` call and left the moment
 * `pocket_status` next reports `paired` - the relay itself exposes no "is a
 * window currently armed" read as its OWN state, only alongside the device
 * view (`phone_window`/`watch_window` below).
 *
 * `"paired"` now carries BOTH slots independently: `Connect` always arms the
 * phone's and the watch's pairing windows together (one QR, both codes), so
 * a partial state (one slot set, the other `null`) means that device was
 * disconnected on its own, never that it was simply not offered a code yet -
 * see `PocketView.tsx`'s paired-card rendering for how each slot is shown.
 *
 * Both `"idle"` and `"paired"` carry `phone_window`/`watch_window`: the
 * phone commonly redeems its code within seconds of a scan while the
 * watch's redemption waits on a WatchConnectivity handoff that can take
 * longer, so `{ state: "paired", phone: {...}, watch: null }` with the
 * watch's window still armed underneath it is a real, ordinary sequence. */
export type PocketStatus =
  | {
      state: "idle";
      cloud_ready: boolean;
      phone_window: PocketWindow | null;
      watch_window: PocketWindow | null;
    }
  | {
      state: "paired";
      phone: PocketDevice | null;
      watch: PocketDevice | null;
      phone_window: PocketWindow | null;
      watch_window: PocketWindow | null;
    }
  | { state: "relay_unreachable"; message: string };

/** Mirrors `pocket::commands::PocketQrDto` - `pocketConnect()`'s success
 * return. `qr_content` is the EXACT `genaryx-pocket://pair/v1?...` string
 * (docs/PHASE5.md W2), now carrying both the phone's and the watch's codes;
 * render it verbatim, never reconstruct it here. */
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
