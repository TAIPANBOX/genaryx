# Phase 5 · W4 — sim end-to-end exit gate (D12) — RESULTS

Status: **PASSED** (simulator, no Apple account), 2026-07-18.

D12 is "TokenFuse Pocket as a remote exception-pager, reaching the money-plane
only through a headless 24/7 `genaryx-relay`, single-device, QR-paired". W4 is
the on-simulator proof that the whole chain works end to end. This file is the
evidence record; the build contract is [PHASE5.md](PHASE5.md).

## The stand

- **TokenFuse Cloud** (money-plane) on `127.0.0.1:8080`, two org keys:
  `devkey` = `default:admin:paid`, `relaykey` = `default:viewer:paid`.
  Seeded small on purpose (`live-campaign/scripts/gx_mobile_seed.py`):
  **$41.08 across 35 runs**, a handful over cap — the exception-first queue is
  meant to be read at a glance, not to page a 9k-row fleet.
- **genaryx-relay** on `127.0.0.1:8443` (public, TLS) + `127.0.0.1:8444`
  (admin, loopback only). Cloud key = the **viewer** key. Public listener
  SPKI-SHA256 pin = `1XP2Nf1uQkCQ7QX42lFfYPlblc0ak1NZ76F6AIYQpWg=` (the QR's
  trust root). Sim deltas per PHASE5.md: APNs = `NullSender`, license gate
  bypassed, software P-256 signing (no Secure Enclave on the sim).
- **TokenFuse Pocket** on the **iPhone 17** simulator (iOS 26.5), paired to the
  relay by pasting the `genaryx-pocket://pair/v1?...` link; TLS pinned to the
  relay's SPKI hash from the QR itself.

## Gate 1 — a signed kill travels phone → relay → Cloud, verified end to end

1. On the phone: tap **Kill** on `support-tier2-bot-000` (117% of cap, `killed=false`).
2. Confirm dialog → **Face ID** (Simulator ▸ Features ▸ Face ID ▸ Matching Face).
3. The phone signs the request (ES256 over the canonical string) and POSTs it to
   the **relay**, which forwards it **verbatim** to the Cloud (no
   re-canonicalization; `proxy.rs::mutation_passthrough`).
4. Result: `support-tier2-bot-000` flips **`killed=False → killed=True`** at the
   Cloud; the phone's row flips to the **KILLED** pill.
5. Relay log confirms it observed the kill over its own SSE stream:
   `genaryx-relay: would push (no APNs token on file): Run killed - Agent run support-tier2-bot-000 was killed`.

### Why this proves a *device-signed* kill (not the relay acting on its own)

The relay holds only a **viewer** key and cannot sign mutations — structural, by
design (D12.3 trust boundary). Demonstrated live:

| Actor | Request | Result |
|---|---|---|
| Relay's **viewer** key | `POST /v1/runs/underwriting-copilot-002/kill` | **HTTP 403**, run stayed `killed=false` |
| Phone (**device ES256**, via relay) | `POST /v1/runs/support-tier2-bot-000/kill` | **200**, run `killed=true` |

A viewer key is refused; the phone's kill succeeded. The only path that kills is
a device signature the **Cloud** verified. The relay is a conduit, never an
authority.

## Gate 2 — single device, freed only on Disconnect

- While the iPhone 17 (`device 0b63f847…`) is paired, arming a second pairing
  window → **HTTP 409 `device_exists`** (a second phone cannot pair).
- Admin **Disconnect** (`POST /admin/disconnect`, the desktop Pocket panel's
  "Disconnect" button) → `was_paired=true`, slot freed (`paired=false`).
- Arming a pairing window now → **HTTP 200**. "One device at a time, released on
  Disconnect."

## Gate 3 — a disconnected phone genuinely loses access (no auth gap)

The read path `GET /relay/v1/exceptions` verifies the device bearer
(`exceptions.rs::exceptions_handler` → `registry.verify_bearer`). After
Disconnect the device row is gone, so:

- `GET /relay/v1/exceptions` with **no bearer** → **401**.
- …with an **unknown/deleted** bearer (an orphaned device's token) → **401**.

## Fix shipped during the gate — honest deauthorization on the phone

**Found:** after a server-side Disconnect the phone kept showing a *frozen*
last-known-good queue behind a green "connected" dot — a failed poll changed no
observable state, so the view simply stopped re-rendering. Misleading: the
operator disconnected it, but the phone still looked live.

**Fixed** (`tokenfuse-mobile`, `ExceptionQueueView.swift`): the store now treats
a **401** as `deauthorized` and the view falls back to the **Connect** screen; a
**transient** failure (network drop, 5xx) still keeps the last-known-good
snapshot exactly as before. Kill/ack mid-action also fall back to Connect on 401.

**Verified on-device:** rebuilt + reinstalled → the stale Keychain session hit a
401 on first poll → app returned to the Connect screen (not frozen data) →
re-paired via a fresh QR link → live queue restored. Regression test added:
`Tests/ExceptionQueueStoreTests.swift` (401 → deauthorized; 5xx → last-known-good).

## Everything on the simulator, no Apple account

No APNs entitlement, no provisioning: pushes are logged by `NullSender`, the
license gate is bypassed for the sim, and signing uses a software P-256 key.
The APNs sender and the ML-DSA offline-license gate are R1 (need an Apple
developer account), tracked in [PHASE5.md](PHASE5.md).
