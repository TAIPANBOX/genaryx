# Phase 6 · C0 (Felyx copilot, credible first cut) — RESULTS

Status: **DONE (desktop, both shells), all gates green**, 2026-07-19. Build contract:
[PHASE6.md](PHASE6.md). Architecture: `itrat-console/13` D13.

C0 is the read-only first cut: the provider abstraction, the loopback residency gate, the
typed read-tool registry over the existing connectors, the hand-rolled agent loop, and a
chat pane in both shells that answers with tool-computed numbers. No proposals, no relay
(those are C1-C3).

## W1 — `crates/copilot` (orchestrator-owned)

New workspace member. Modules: `provider` (the `LlmProvider` trait + `OpenAiCompat` +
`AnthropicMessages` + a test `MockProvider`), `residency` (the gate), `config`
(`[copilot]` block + `env:`/`file:` secret refs), `tools` (the registry + 10 read tools),
`agent` (Felyx, the loop), `action` (`ProposedAction`, defined now, emitted in C2),
`service` (`CopilotService`, the one assembly entry the shells call).

**The three-line safety model is structural, from day one:**
- `crates/copilot` has **no dependency on `genaryx-signing`** and holds no signer — it
  cannot produce an `X-Fuse` signature, so it cannot act. Enforced by
  `tests/no_signer.rs`, which parses the crate's own `Cargo.toml` dependency tables and
  fails if the signer ever appears.
- Read tools execute; `Propose` returns a `ProposedAction` object (C2); `Act` does not
  exist as a code path.
- The **residency gate** (`residency.rs`): every real provider constructor refuses a
  non-loopback / non-RFC1918 / non-link-local `base_url` while
  `allow_non_local_endpoints = false` (the default). A sensitive install cannot leak by
  misconfiguration; the provider client is the only egress and it is pinned local.

**The 10 read tools** (all real connector methods): `money_summary`, `list_runs`,
`list_agents`, `savings`, `incidents`, `alerts` (CloudClient); `identities`,
`identity_alerts` (IdryxClient); `policies`, `approvals_inbox` (WardryxClient). Each is a
thin wrapper returning the connector DTO as JSON; a tool whose plane is not configured is
simply not advertised. The registry is fixed and typed — no tool synthesis, no shell, no
URL fetch (the prompt-injection floor).

**Connector change (mine, minimal):** added `Serialize` to the money and policy read DTOs
(`Summary`, `RunAgg`, `AgentAgg`, `SavingsSummary`, `Alert`, `Incident`, `Severity`,
`Approval`, `PolicyRecord`) so the tools can emit them as JSON. Deserialize-only before;
additive, nothing else changed. Idryx DTOs were already `Serialize`.

### W1 tests (deterministic + live)

- 28 crate unit tests + `no_signer` guard. Cover: residency classification (loopback /
  RFC1918 / IPv6 ULA accepted; public IP + public DNS refused), config + secret-ref
  parsing, provider request-shaping + response-parsing for BOTH wire formats (OpenAI
  `tool_calls`, Anthropic `tool_use`/`tool_result` folding), the loop over a `MockProvider`
  (text-only turn, tool-call-then-answer, iteration-bound), and the disabled-service path.
- **Live e2e (skip-graceful):** against the seeded TokenFuse Cloud on `127.0.0.1:8080`,
  the loop ran the REAL `alerts` tool and the result flowed back to the model:
  `live e2e OK: alerts tool returned [{"budget_micros":1260000,"fraction":0.87…,"run_id":"aml-case-copilot-003",…}, …]`.
  This is the C0 promise made concrete: numbers come from a tool, not the model.

## W2 — chat pane in both shells (two Sonnet tracks, orchestrator-reviewed)

Both add a **Copilot** tab with a chat pane, a **residency banner** (green "local: … via
Ollama" / amber "remote: …, BYO key" / muted "no provider configured"), and a per-answer
"tools used" disclosure rendering the `tool_trace` verbatim (the evidence surface).

- **Tauri** (`apps/desktop`): `src-tauri/src/copilot/` (`copilot_status` + `copilot_ask`
  commands over `CopilotService`), `CopilotView.tsx`, registered in `views.ts`/`AppShell`.
- **SwiftUI** (`crates/ffi` + `apps/macos`): a `CopilotHandle` UniFFI Object (owns a tokio
  runtime, `block_on`, `#[derive(uniffi::Error)]`, fail-closed) + `CopilotView.swift` /
  `CopilotModel.swift`, registered in `GenaryxApp.swift`.

Both keep the shells thin: every brain cell is Rust in `crates/copilot`; the panes are UI.

## Exit gate (C0)

1. **Loop + tools** proven deterministically (mock provider) and live (real `alerts` read
   flowed back to the model) — above.
2. **Residency gate** refuses a public endpoint with `allow_non_local = false` and accepts
   it only when explicitly opted in — unit + `CopilotService` tests.
3. **Structural "cannot act"** — `no_signer.rs` asserts the crate never depends on the
   signer.
4. **Both shells build and render** the Copilot pane + residency banner — gates below +
   screenshot.

**All gates green** (re-run by the orchestrator on the integrated tree): `cargo fmt --check`,
`cargo clippy --workspace --all-targets -D warnings`, `cargo test --workspace` (25
test-suites), Tauri `tsc --noEmit` + `pnpm build` + `src-tauri cargo build`, SwiftUI
`build-ffi.sh` + `swift build`.

## Sim-first deltas (no local LLM on the build box, no Apple account)

- **No inference runtime here**, so the default provider is `none` and the shells show the
  honest "no provider configured" state. Correctness is proven by the `MockProvider` and
  the live-tool e2e; real inference is a config-time choice (Ollama / LM Studio locally, or
  a BYO-key cloud provider with the residency opt-in), exactly as APNs was a `NullSender`
  seam in D12.
- **No macOS Keychain in this codebase**, so `api_key_ref` resolves via `env:VAR` or
  `file:/path` (0600) like every other Genaryx secret; the spec's `keychain:` scheme is a
  later hardening pass.

## Next (C1-C3, not built here)

- **C1** triage + explanation: incident "explain" cards, cross-plane root-cause from the
  core store/graph, Engram recall (sync bridge), qryx/verdryx attestation tools.
- **C2** propose-and-confirm: `ProposedAction` through the Wardryx `decide` pre-check and
  the existing signed ceremony; copilot self-budget via the local TokenFuse gateway
  (`run_id = genaryx-copilot`); audit metadata "human X approved copilot proposal Y".
- **C3** the D12 tie: a triage stage inside `genaryx-relay`, soft-queue batching + push
  annotation within the 3 s latency budget, HARD-events-always-push floor, trial-mode lock.
- **Wiring**: thread the existing shell connector clients into `genaryx_copilot::Clients`
  so Felyx's tools have real data in-shell (today the tools are proven via the crate's own
  CloudClient; the shells pass `Clients::default()`).
- **A live LLM demo** (Ollama locally, or a BYO Anthropic/OpenRouter key) — needs either an
  install or a key + spend opt-in.
