# Phase 5 - Distance to the pocket (D12: remote mobile + headless relay), sim-first

Source of truth for the architecture: `~/Development/itrat-console/13-mobile-relay-copilot-decision.md`
(D12 sections) + the requirements brief `12-mobile-relay-requirements.md`. This doc is the
BUILD contract: the wave plan, the sim-first deltas, and the exit gate. D13 (the Felyx copilot)
is a later phase, not built here.

Everything here is defensive: the relay lets an operator protect their own budget and agents
from anywhere; every mutation stays human-initiated and hardware-signed. The relay is a
least-privilege pipe (viewer key), it cannot kill or mutate on its own (Cloud 403s a viewer,
`tokenfuse http.rs:276-277`).

> ## AMENDMENT, 2026-07-20: the single-device rule is superseded
>
> **Everything below that says the relay accepts exactly ONE device is a historical
> record of Ф5 as it shipped and PASSED. It is no longer how the relay behaves.**
> Read this box before quoting any of it, and especially before writing anything
> public from it.
>
> The relay now accepts exactly **one phone AND one watch**. The Apple Watch became a
> second pager surface for the same operator, which the one-row rule made impossible.
> What changed, precisely:
>
> - `device` gained a `kind` column (`CHECK (kind IN ('phone','watch'))`) plus a
>   `UNIQUE INDEX device_kind_unique`, so "at most one per kind" is now enforced by the
>   SCHEMA, not only by the application checks that guard the races. That is stricter
>   than the V1 arrangement it replaces.
> - `pairing_window` is rekeyed from a singleton `id = 1` row to one row PER KIND.
> - **The kind is bound to the CODE, never claimed by the device.** The desktop declares
>   which slot a code admits when it arms the window; the redeeming device's self-reported
>   `platform` string is never consulted when choosing a slot. A device cannot take the
>   watch slot by calling itself a watch.
> - One QR now carries TWO codes (`code` = phone, `code_watch` = watch). The phone scans
>   it, pairs itself, and forwards the watch its own code over WatchConnectivity, because
>   the watch has no camera and watchOS has no CoreImage to render a QR back.
> - `POST /relay/v1/pair` accepts an optional `expect_kind` and refuses a crossed code
>   BEFORE redeeming it at the Cloud, so a mis-built QR cannot strand a slot.
> - Revocation is per slot: losing the watch does not force the phone to re-pair.
> - `GET /admin/device` is replaced by `GET /admin/devices` (one entry per slot). The path
>   was renamed deliberately so a stale desktop gets a loud 404 instead of mis-parsing a
>   changed body.
> - **The wrist is a narrower surface than the pocket.** Both may kill and acknowledge;
>   only the phone may change a budget (`proxy.rs::kind_may_mutate`, 403
>   `forbidden_for_device`). Note this is enforced AT THE RELAY: the Cloud's role model is
>   only `admin` or `viewer` and cannot express "may kill but may not re-budget", so the
>   watch's credential is not intrinsically weaker at the Cloud. Do not write that it is.
> - `ttl_secs` on a pairing window is now capped at 900s (`MAX_PAIRING_WINDOW_SECS`).
>
> Two things a writer must NOT carry over from the text below:
> the phrase "single-device" as a current property, and the claim that a second pair
> attempt is refused outright. A second attempt is refused only for a slot that is
> already filled.

## Sim-first deltas (no Apple Developer account yet)

The whole D12 dataflow is built and proven on the iOS Simulator + a LOCAL TokenFuse Cloud.
The only thing that genuinely needs the paid Apple account is real APNs remote-push, so:

- **APNs is stubbed to a NullSender in the sim phase.** The phone gets exceptions by POLLING
  `GET /relay/v1/exceptions` (short interval) + on foreground, exactly the "deterministic
  floor / push is only a wake channel" model from 13 D12.3. The `ApnsSender` seam is wired but
  points at a NullSender; swapping in the real (already-existing) `tokenfuse-cloud::apns` is an
  R1 config change once the account exists.
- **Cloud is local** (`taipan up` / the dev harness on this Mac), not a Hetzner box. The relay
  colocates with it over loopback, same as the production loopback path.
- **TLS + SPKI pin still real** on the sim: the relay serves rustls with a self-signed key, the
  QR carries the pin, the app enforces it. This is testable end to end without Apple.
- Device signing on the sim is the software P-256 path (`Crypto.swift` already does software keys
  under the simulator); Enclave is hardware-only and comes with real-device testing later.

## Waves (playbook unchanged: contract-first, security-critical hand-written by the orchestrator,
## routine per-shell tracks in Sonnet, review every diff, all gates, hold push to phase-5-complete)

- **W1 - relay core (Rust, `genaryx/crates/relay`).** New closed workspace member. Config +
  (license gate stubbed permissive in sim, real ML-DSA in R1) + axum+rustls TLS listener +
  SQLite single-device registry + pairing window + `POST /relay/v1/pair` that redeems the SAME
  code against the Cloud (`/v1/pair`) so the phone's pubkey is registered at the Cloud itself
  (relay never becomes a signature authority) + read proxy for the `/v1` read subset + mutation
  pass-through (verbatim forward of `X-Fuse`-signed kill/budget/ack) + ExceptionEngine over
  `CloudSse` with reconcile-on-reconnect + `GET /relay/v1/exceptions`. APNs seam = NullSender.
  Admin API (pairing-window arm, device view, disconnect) on a SEPARATE loopback/WG-only listener.
  Reuse: `CloudSse`, `CloudClient` (reads), `genaryx-signing` verify only, rusqlite. Security-
  critical (pass-through trust, pairing redeem, single-device) hand-written + reviewed by the
  orchestrator. Gate: `cargo build/test -p genaryx-relay`, fmt, clippy -D warnings.
- **W2 - desktop Pocket panel (both shells).** "Connect TokenFuse Pocket" -> mint code at Cloud
  (`/v1/pair/new`, admin key over loopback/WG) -> arm relay pairing window (admin API) -> render
  QR (`genaryx-pocket://pair/v1?relay=...&pin=...&code=...&org=...`) -> show paired device
  (name/platform/paired_at/last_seen) + Disconnect. Tauri (TS) + SwiftUI. Gate: tsc+build; swift build.
- **W3 - mobile (Swift, `tokenfuse-mobile`).** Connect screen -> QR scanner (VisionKit
  DataScanner) -> parse+validate `genaryx-pocket://pair/v1` -> generate device key (software in
  sim) -> pinned-TLS `POST /relay/v1/pair` -> persist session in Keychain (existing SessionStore)
  -> exception-queue screen (triage/exception-first: aggregates + only at-risk/over-cap/runaway/
  pending-approvals, NOT the 9k-run list) fed by `GET /relay/v1/exceptions` (poll) -> existing
  kill/budget UI pointed at the relay base URL (signatures transfer verbatim). Gate: xcodebuild sim.
- **W4 - sim end-to-end exit gate.** Local Cloud seeded (small dataset, like the mobile campaign
  seed) -> relay up over loopback -> desktop mints+renders QR -> sim scans -> pairs (pin enforced)
  -> exception queue shows the real money slice -> slide-to-kill a runaway -> signed request
  passes relay verbatim -> Cloud verifies E2E -> run killed -> exception state flips. Single-device:
  a second pair attempt refuses until Disconnect. All on the simulator, no Apple account.

## Exit gate (Ф5 sim)

> **Status: PASSED (2026-07-18).** Full evidence in [PHASE5-W4-RESULTS.md](PHASE5-W4-RESULTS.md):
> device-signed kill verified phone -> relay (verbatim) -> Cloud (viewer-key kill = 403, so only a
> device ES256 signature could have killed it); single-device 409 -> Disconnect -> 200; read path
> 401s a revoked device (no auth gap). One correctness fix shipped in `tokenfuse-mobile`: the phone
> now returns to Connect on a 401 instead of silently freezing on last-known-good data.

On the simulator against a local Cloud: a phone pairs by scanning a QR off the desktop (zero
manual entry, SPKI pin enforced), sees ONLY the exception slice (never the full fleet), and
performs a hardware-path-signed (software key in sim) kill that travels relay -> Cloud verbatim
and is verified end to end (audit actor = `device:<id>`), with single-device binding enforced and
Disconnect freeing the slot. APNs remains a NullSender seam (real push is R1 + Apple account).

## Deferred to R1+ (not in the sim exit gate)
Real APNs (existing `tokenfuse-cloud::apns`, needs Apple account), redacted-payload mode, relay
rate-limit/lockout + audit log, `taipan`-CLI packaging + systemd unit, `DELETE /v1/devices/{id}`
+ `/v1/runs` pagination upstream PRs, Watch companion, HA relay. Real-device (7-day) install.
