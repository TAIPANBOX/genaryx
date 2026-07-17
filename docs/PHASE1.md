# Phase 1 — money + deploy

Source: `itrat-console/09` Ф1. Builds on Phase 0 (core + both shells live over
the shared core). Estimate: 2 sessions. Branch `phase-1-money`.

**Exit gate (killer demo, 09 §6).** `taipan up` lays down a stack -> the console
self-discovers the environment (descriptor) -> both shells show real Cloud data
(Overview + Money) -> a runaway agent trips the breaker (402) -> the operator
kills the run with a hardware-signed (Secure Enclave) mutation the Cloud accepts
-> the `console_command` event appears on the bus. Runs end to end.

## Scope

- [x] Cloud REST connector (`crates/connectors`): typed client for
      summary/runs/agents/savings/series/incidents/alerts/audit(+verify); reuse
      the Phase-0 `CloudSse` for the live ticker; ES256-signed mutations
      (kill/budget/ack) via `es256`+`enclave` (spike #2), device-pairing.
      **Status:** `CloudClient` (`crates/connectors/src/cloud_rest.rs`) reads
      summary/runs/agents/savings/incidents/alerts/audit-verify and signs
      kill/budget/ack via `genaryx-signing::es256` - works with any
      `Es256Signer`, `enclave::SecKeySigner` included, unchanged; pairing
      (`pair/new` + `pair`) implemented. `series` deferred - a chart-panel
      concern, not part of the wave-1 data spine. Live-proven against a real
      `tokenfuse-cloud`: pair, reads, a signed kill, a signed budget change,
      and a tampered signature rejected `403` - see
      `crates/connectors/tests/cloud_rest_test.rs`.
- [ ] CommandBroker (`crates/core`): draft -> precheck -> (Wardryx decide, ENT)
      -> sign -> execute -> journal + emit `console_command`; fail-closed,
      break-glass ceremony.
- [ ] Overview + Money panel, BOTH shells: live burn, runs (budget/spent/steps/
      killed), savings, incidents (+ack), kill + budget change (hardware-signed).
- [ ] menu-bar mini: burn rate + kill last runaway (SwiftUI NSStatusItem, Tauri tray).
- [ ] `taipan up` v0: separate OPEN repo `TAIPANBOX/taipan`, native process
      supervisor (generalize `bank-in-a-box`), port map (gateway 4100, Cloud 8080,
      Idryx 8081, Wardryx 8090), events dir, keys, descriptor + console auto-discovery.
- [ ] Killer demo (09 §6) as an e2e acceptance test.

## Cloud API (verified from `~/Development/tokenfuse/crates/cloud/src`)

- Read: `/v1/summary` `/v1/runs` `/v1/agents` `/v1/savings` `/v1/series`
  `/v1/stream` (SSE) `/v1/incidents` `/v1/alerts` `/v1/audit` `/v1/audit/verify`
  `/v1/audit/manifest` `/v1/compliance(+/evidence)` `/v1/budgets` `/v1/kills`
  `/v1/replay/{run}`.
- Mutations (admin, ES256 device-signed): `POST /v1/runs/{run}/kill`,
  `POST /v1/runs/{run}/budget`, `POST /v1/incidents/{id}/ack`.
- Pairing: `POST /v1/pair/new` (admin) -> `POST /v1/pair` (device pubkey ->
  device_token). Auth: bearer `key:org[:role][:plan]`, fail-closed; devkey via
  `TOKENFUSE_CLOUD_ALLOW_DEVKEY=1`. Signed-request headers + canonical: proven in
  spike #2 (`crates/signing/src/{es256,enclave}.rs`).

## Waves (Opus orchestrates + reviews every diff; Sonnet implements; Fable by permission)

1. **Cloud REST connector + signed mutations** (`connectors`) - the Money-panel data spine.
2. **CommandBroker** (`core`) - the signed-mutation lifecycle + `console_command`.
3. **Overview + Money panels**, both shells (Sonnet Web + Sonnet SwiftUI from one spec).
4. **`taipan up` v0** (new open repo `TAIPANBOX/taipan`) + descriptor auto-discovery.
5. **menu-bar mini** + **killer-demo** e2e acceptance.

Reuses Phase-0 proofs directly: `CloudSse` (spike #6), `es256`/`enclave` signing
+ live pairing (spike #2), `IngestService`/`Store` (the bus + `console_command`
sink), both shell Bus Explorers (Money is a sibling panel).
