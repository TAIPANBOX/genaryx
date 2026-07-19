# Phase 6 · C2 — Felyx propose-and-confirm

Build contract for C2, extending [C0](PHASE6-C0-RESULTS.md) / [C1](PHASE6-C1-RESULTS.md).
Architecture: itrat-console/13 D13.3 + D13.7 C2 — "`ProposedAction` cards through Wardryx
pre-check + the existing signed ceremony; copilot self-budget via the local TokenFuse
gateway; audit metadata linking proposal to human approval".

C0/C1 shipped read + explain. C2 lets Felyx **recommend an action** without ever performing
it. The safety spine is unchanged and is the whole point:

> An AI cannot press the buttons (no signer) and an AI cannot silence the pager (deterministic
> floor). A proposal is display data; a human signs.

## C2-W1 — crate (orchestrator-owned)

### Propose tools (the copilot recommends, never acts)
Four parameterized tools, each builds a [`action::ProposedAction`] and returns it — they hold
NO connector-mutation call and NO signer (structural, `no_signer.rs` still guards it):

| Tool | Args | ProposedAction |
|---|---|---|
| `propose_kill` | `{run_id, reason, confidence?, evidence_refs?}` | `kind=Kill, target=run_id` |
| `propose_budget` | `{run_id, usd_cap, reason, …}` | `kind=Budget, params={usd_cap}` |
| `propose_grant_deny` | `{approval_id, verdict:"grant"|"deny", reason, …}` | `kind=GrantDeny, params={verdict}` |
| `propose_rescan` | `{reason, target?, …}` | `kind=Rescan` |

A propose tool overrides `Tool::is_propose() -> true`. The loop, after dispatching a propose
tool, deserializes its result into a `ProposedAction` and collects it into
**`Answer.proposals: Vec<ProposedAction>`** (a new field), so the shell renders proposals as
cards distinct from the free text. The result also goes back to the model as data (so it
knows the proposal is queued and stops).

### Wardryx pre-check (side-effect-free)
`Wardryx /v1/decide` can CREATE an approval hold as a side effect, so it is NOT safe for a
dry-run. C2's pre-check is therefore a **read**: when Wardryx is configured, a propose tool
calls `list_policies` and attaches the governing policy targets to the proposal
(`ProposedAction.policy_context: Vec<String>`, `#[serde(default)]`), so the card can show
"this action is governed by policy X". The precise binary allow/deny PDP dry-run is
**deferred** — it needs a genuine dry mode on Wardryx `/v1/decide` (an upstream enhancement),
noted rather than faked with a side-effectful call.

### Copilot self-budget (D13.3)
Add `run_id` (default `"genaryx-copilot"`) to the provider config; the provider clients send
it as an `x-fuse-run-id` header on every request. When the operator points `base_url` at
their local TokenFuse gateway's LLM proxy, the copilot's own inference spend is attributed to
`run_id=genaryx-copilot` and capped by the Breaker exactly like any other agent (the thesis,
dogfooded). Harmless when `base_url` is a raw Ollama/Anthropic endpoint (ignored).

### Tests (deterministic)
- A MockProvider calls `propose_kill`; the loop surfaces a `ProposedAction` in
  `Answer.proposals` with the right kind/target and (Wardryx absent) empty `policy_context`.
- `no_signer.rs` still passes (propose tools produce descriptors, not signatures).
- The provider request carries the `x-fuse-run-id` header (checked via the request-shaping
  unit path).

## C2-W2 — both shells (two Sonnet tracks)

- **Render proposal cards** from `Answer.proposals`: kind, target, params, rationale,
  confidence, evidence_refs, and a "governed by policy …" note when `policy_context` is
  non-empty.
- **"Approve" routes into the EXISTING signed ceremony** the shell already has from Phases
  1-3: `Kill`/`Budget` via the Money path (desktop enclave / Touch-ID signature),
  `GrantDeny` via the Policy approvals path, `Rescan` via the Identity rescan. The copilot
  crate produced only the descriptor; the human's signature is what executes it. REUSE that
  code — do not reimplement signing.
- **Audit metadata**: on a successful approve, journal the link
  (`console.copilot_proposal_approved {proposal_id, kind, target}`) alongside the signed
  mutation, so the record reads "human X approved copilot proposal Y", never "copilot did Z".
- A "Reject"/dismiss simply drops the card.

## Deferred (post-C2)
- The precise Wardryx PDP dry-run verdict (needs a Wardryx dry mode).
- The false-alarm memory write-back (from C1).
- C3 (the relay triage tie), unchanged from PHASE6.md.

## Gates (unchanged)
`cargo fmt --check`, `cargo clippy --workspace --all-targets -D warnings`,
`cargo test --workspace`, Tauri `tsc` + `pnpm build` + `src-tauri cargo build`, SwiftUI
`build-ffi.sh` + `swift build`.
