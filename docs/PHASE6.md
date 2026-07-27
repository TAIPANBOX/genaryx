# Phase 6 - Felyx, the Genaryx AI copilot (D13), C0 first

BUILD contract for D13, the read + propose (never act) copilot. Architecture source of
truth: `~/Development/itrat-console/13-mobile-relay-copilot-decision.md` (D13.1-D13.7).
This file is the wave plan + the exact reuse map + the sim-first deltas for **C0**, the
credible first cut. C1-C3 are outlined at the end, not built here.

Name: **Felyx** (resolved with Yurii, 2026-07-18).

## What C0 is (D13.7)

Desktop-only, read-only, no proposals, no relay: the provider abstraction, the loopback
residency gate, the typed read-tool registry over the existing connectors, a small
hand-rolled agent loop, and a chat pane in both shells that answers natural-language
questions with **tool-computed numbers** (the model never does arithmetic in prose).

The safety spine is in from day one even though C0 has no proposals:
- `crates/copilot` has **no dependency on `genaryx-signing`** and holds no signer. It is
  structurally unable to produce an `X-Fuse` signature. (CI-checkable: grep the crate's
  Cargo.toml.) This is the "an AI cannot press the buttons" guarantee (D13.4).
- The action model is three-tier in the type system: `Read` executes, `Propose` returns a
  `ProposedAction` object (defined in C0, emitted from C2), `Act` does not exist.

## Sim-first deltas (no local LLM on the build box, no Apple account)

- **No inference runtime here.** Ollama/LM Studio/vLLM are the client's to run (D13.1: "we
  never build or embed inference"). So the default provider is `none`, and correctness is
  proven with a **`MockProvider`** (test-only) that scripts tool-calls then a final answer.
  The two real provider clients (OpenAI-compatible + Anthropic) are exercised against a
  **fake HTTP server** for wire-format shaping/parsing (same technique as the relay's
  `spawn_fake_cloud` in `proxy.rs` tests). Real inference is a config-time choice, exactly
  like APNs was a `NullSender` seam in D12.
- **No macOS Keychain in this codebase.** Extractor confirms every existing handle reads
  secrets from a `~/.taipan` descriptor ref or an env var (`crates/ffi/src/*/env.rs`), not
  Keychain. So the copilot resolves `api_key_ref` the same way: `env:VAR_NAME` or
  `file:/abs/path` (0600). The spec's `keychain:` scheme is deferred to a later hardening
  pass; C0 uses env/file to match the repo.

## Crate layout (C0-W1, orchestrator-owned)

New workspace member `crates/copilot` (add to `Cargo.toml` members). Deps: `genaryx-connectors`,
`genaryx-core` (DTO/JSON only), `serde`, `serde_json`, `thiserror`, `tokio`, `reqwest`,
`chrono`. **Never** `genaryx-signing`.

```
crates/copilot/src/
  lib.rs         // re-exports; module docs stating the read/propose/never-act model
  config.rs      // [copilot] block: provider, base_url, model, api_key_ref,
                 //   allow_non_local_endpoints (default false), max_usd_per_day
  residency.rs   // is_local_endpoint(url): loopback + RFC1918 + link-local only;
                 //   the constructor gate; hand-written + exhaustively unit-tested
  provider/
    mod.rs       // trait LlmProvider { async chat(ChatRequest)->ChatTurn; descriptor() }
                 //   + ChatRequest/ChatTurn/Message/ToolCall/Usage/ProviderDescriptor/
                 //   ProviderError; the residency gate is enforced in every real ctor
    openai.rs    // OpenAiCompat { base_url, api_key: Option<SecretRef>, model }
    anthropic.rs // AnthropicMessages { base_url, api_key: SecretRef, model }
    mock.rs      // #[cfg(test)] MockProvider: scripted turns for deterministic loop tests
  tools/
    mod.rs       // trait Tool { name; description; params_schema; async run(args)->Value }
                 //   + ToolRegistry (typed dispatch, fixed set, no synthesis)
    cloud.rs     // money_summary,list_runs,list_agents,savings,incidents,alerts
    idryx.rs     // identities, identity_alerts
    wardryx.rs   // policies, approvals_inbox
  agent.rs       // Felyx: system prompt assembly (delimited provenance-tagged data
                 //   blocks declared as data, never instructions), the bounded loop
                 //   (provider.chat -> execute tool_calls -> feed back -> repeat),
                 //   token/cost budget accounting, max_iterations
  action.rs      // ProposedAction {kind,target,params,rationale,confidence,evidence_refs}
                 //   + ActionKind enum. Defined now, emitted in C2. No Act variant.
```

### The residency gate (security-critical, hand-written)

`is_local_endpoint(&url) -> bool` accepts only: `127.0.0.0/8`, `::1`, `localhost`, the
RFC1918 ranges `10/8` + `172.16/12` + `192.168/16`, and link-local `169.254/16` +
`fe80::/10`. Every real provider constructor takes `allow_non_local_endpoints: bool`; when
`false` (default) and the `base_url` host is not local, the constructor returns
`ProviderError::NonLocalEndpointRefused` before any client is built. A sensitive install
cannot leak by misconfiguration. Unit tests pin: loopback/RFC1918 accepted, public IP +
public DNS refused, `allow_non_local=true` lets a public endpoint through (BYO-cloud path).

### The tool registry (C0: 10 async read tools, all real methods)

Every tool is a thin typed wrapper over an existing connector read method; the tool's
`run` calls it and returns `serde_json::to_value(dto)`. No free-form anything.

| Tool | Backing method | Client |
|---|---|---|
| `money_summary` | `summary()` | CloudClient |
| `list_runs` | `runs()` | CloudClient |
| `list_agents` | `agents()` | CloudClient |
| `savings` | `savings()` | CloudClient |
| `incidents` | `incidents()` | CloudClient |
| `alerts` | `alerts()` | CloudClient |
| `identities` | `list_identities()` | IdryxClient |
| `identity_alerts` | `list_alerts()` | IdryxClient |
| `policies` | `list_policies()` | WardryxClient |
| `approvals_inbox` | `list_approvals()` | WardryxClient |

Sync clients (Qryx, Verdryx, Engram `&mut self`) and Wardryx `decide` (dry PDP explain)
are C1/C2, where the loop grows a `spawn_blocking` bridge for the sync ones.

### The agent loop (D13.1)

Small, hand-rolled, no framework. `Felyx::answer(question) -> Answer { text, tool_trace,
usage }`. Loop: assemble system prompt + the question; `provider.chat` with the tool specs;
if the turn has tool-calls, dispatch each through the registry, append results as tool
messages (tagged as DATA), iterate; else return the text. Bounded by `max_iterations`
(default 6) and a token budget. The `tool_trace` (which tools ran, with the row ids they
returned) is returned so the shell can render evidence next to the model text (the
anti-hallucination surface: numbers come from tools, shown verbatim).

## C0-W2 - chat pane in both shells (two Sonnet tracks, orchestrator reviews)

Non-overlapping zones, exactly as Phases 0-5.
- **Track A (Tauri, `apps/desktop`):** `#[tauri::command]`s (`copilot_descriptor`,
  `copilot_ask`) over `crates/copilot`; a React `CopilotView.tsx` chat pane with the
  **residency banner** ("local: … via Ollama" vs "remote: …, BYO key" vs "no provider
  configured"), the message list, and a tool-trace disclosure; register the tab.
- **Track B (SwiftUI, `crates/ffi` + `apps/macos`):** a `CopilotHandle`
  (`#[derive(uniffi::Object)]` + `#[uniffi::export]`, owns a tokio runtime, `block_on`,
  `#[derive(uniffi::Error)]`) exposing `descriptor()` + `ask()`; `CopilotView.swift` +
  `CopilotModel.swift` chat pane + residency banner + tool-trace; register the tab.

## C0-W3 - exit gate (verifiable without a real LLM)

1. **Deterministic loop e2e** (`crates/copilot/tests`): a `MockProvider` scripts "call
   `alerts`, then answer". The loop runs the REAL `alerts` tool against a seeded local
   Cloud (skip-graceful if absent), and the final answer carries the tool-computed count,
   proving numbers flow from tools, not the model.
2. **Residency gate**: unit tests + one integration check that a public `base_url` is
   refused with `allow_non_local=false` and accepted with it `true`.
3. **Provider wire format**: OpenAiCompat + AnthropicMessages request-shaping and
   response-parsing verified against a fake HTTP server.
4. **`genaryx-signing` absence**: a test (or CI grep) asserts the crate does not depend on
   the signer.
5. **Shell**: a screenshot of the Copilot pane + residency banner rendering in a shell.

All gates: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -D warnings`,
`cargo test --workspace`, Tauri `tsc --noEmit` + `pnpm build`, SwiftUI `build-ffi.sh` +
`swift build`.

## Deferred to C1-C3 (not in C0)

- **C1** triage + explanation: incident "explain" cards, cross-plane root-cause chains from
  the core store/graph, Engram recall (`memory_recall`/`memory_why`, sync bridge),
  false-alarm memory loop, qryx/verdryx `attestation_status`.
- **C2** propose-and-confirm: `ProposedAction` cards through the Wardryx `decide` pre-check
  and the EXISTING signed ceremony; copilot self-budget via the local TokenFuse gateway
  (`run_id = genaryx-copilot`, the Breaker 402 stops a runaway copilot); audit metadata
  "human X approved copilot proposal Y".
- **C3** the D12 tie (after relay R1): a triage stage inside `genaryx-relay` calling
  `crates/copilot`, soft-queue batching/digests, push annotation within the 3 s latency
  budget with the HARD-events-always-push deterministic floor, morning summary; trial-mode
  lock (local providers only).

## Tier (D13.5)

Genaryx only. The crate lives in this workspace; the open stack stays
fully operable without it (the copilot only consumes public plane APIs, adds none). The
trial ships the copilot hard-locked to local providers (`allow_non_local_endpoints=false`).

## I10 - Felyx optimization recommendations (post-C3 addition)

Gives Felyx two more READ tools so it can analyze TokenFuse cost/savings and recommend
optimizations, without adding any new action kind or touching enforcement:

- `savings_breakdown` - blocked/cache/router savings and budget breaks, read straight off the
  local TokenFuse Parquet trace via `tokenfuse savings`
  (`crates/connectors/src/tokenfuse.rs::TokenfuseClient::savings`). Overlaps in SHAPE with the
  pre-existing `savings` tool (`tools/cloud.rs`, sourced from Cloud's own `/v1/savings`
  ledger) - a deliberate, flagged duplication rather than a silently-reconciled one:
  `savings_breakdown` still works when Cloud is not configured, and doubles as a cross-check
  against it.
- `cost_per_action` - cost, call count, and tool-call totals per model and per agent, via two
  FIXED `tokenfuse sql` aggregate queries (`TokenfuseClient::cost_per_action`). Honest about
  the `tool_calls` column being nullable (I1): a row reports `tool_calls_known_rows == 0`
  rather than fabricating a zero when the underlying trace predates that column, and
  `cost_per_tool_call_microusd` is `null` in exactly that case.

Both tools are strictly READ. An "enable the semantic cache" or "route to a cheaper model"
recommendation can only ever be informational text in Felyx's answer - the console has no way
to mutate gateway config, and this crate still holds no signer (`crates/copilot/tests/
no_signer.rs` still passes unchanged). A recommendation that DOES map to an existing action
(e.g. "cap this wasteful agent's budget") is left entirely to the model's own judgment to raise
via the EXISTING `propose_budget` tool after reading these numbers - neither new tool
auto-emits a proposal.

Wiring: `crates/copilot/src/tools/optimize.rs` (the two tools, over the same `spawn_blocking`
bridge `crypto_scan` established for a CLI connector); `Clients::tokenfuse: Option<
TokenfuseTraces>` (`tools/mod.rs`) gates their registration; `crates/api/src/copilot/
state.rs::resolve_tokenfuse` resolves the bin+traces-dir pair by reusing the Evidence Center's
OWN `evidence::env::discover_tokenfuse` - no new binary-path convention was needed, Evidence
already resolves `~/.taipan/bin/tokenfuse-gateway` plus the newest `<name>.traces/gateway`
dir.

Parse fragility, stated plainly: `tokenfuse sql` has no JSON/machine-readable output mode as of
this reading - it only ever prints DataFusion/Arrow's `pretty_format_batches` box-drawing
table, so `cost_per_action` text-scrapes that table. To keep the blast radius of that fragility
small, the two SQL queries are FIXED constants in `tokenfuse.rs`, never operator- or
model-supplied text - `cost_per_action` takes no arguments at all. `tokenfuse savings`'s
human-readable report is scraped the same way. Both parses hard-fail
(`TokenfuseError::Parse`) rather than guess when the shape does not match; validated
2026-07-23 against a live `tokenfuse-gateway` binary and two real local trace directories
(`crates/connectors/src/tokenfuse.rs`'s tests embed the captured bytes, plus a skip-graceful
live test that re-runs the real CLI end to end when that binary/trace happen to be present).
