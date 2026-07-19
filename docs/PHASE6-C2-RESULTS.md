# Phase 6 · C2 (Felyx propose-and-confirm) — RESULTS

Status: **DONE (both shells), all gates green**, 2026-07-19. Contract:
[PHASE6-C2.md](PHASE6-C2.md). Builds on [C0](PHASE6-C0-RESULTS.md) / [C1](PHASE6-C1-RESULTS.md).

C2 lets Felyx **recommend an action** and a human **approve + sign** it. The safety spine is
the whole point and is unchanged: the copilot holds no signer, so it produces a descriptor,
never a mutation; the human's existing signed ceremony is what executes it.

## W1 — crate (orchestrator-owned)

- **Propose tools** (`propose_kill`, `propose_budget`, `propose_grant_deny`, `propose_rescan`):
  parameterized tools that build a `ProposedAction` and return it. They hold no
  connector-mutation call and no signer (`no_signer.rs` still guards it). Marked
  `Tool::is_propose() -> true`; registered unconditionally (a proposal is a descriptor, not a
  plane read, so the copilot can recommend even where a plane is unconfigured).
- **`Answer.proposals: Vec<ProposedAction>`**: the loop deserializes each propose-tool result
  into a `ProposedAction` and collects it, so the shell renders proposals as approve/reject
  cards distinct from the free text. The result also returns to the model as data (so it
  knows the proposal is queued and stops).
- **Wardryx pre-check (side-effect-free)**: `ProposedAction` gained `policy_context:
  Vec<String>` (`#[serde(default)]`). When Wardryx is configured, a propose tool reads
  `list_policies` and attaches the governing policy targets, so the card shows "governed by
  policy X". The precise binary allow/deny PDP dry-run is **deferred**: Wardryx `/v1/decide`
  can create an approval hold as a side effect, so it is not safe to call for a dry-run
  (noted, not faked).
- **Copilot self-budget (D13.3)**: config gained `run_id` (default `genaryx-copilot`); the
  provider clients send it as an `x-fuse-run-id` header on every request. Point `base_url` at
  the local TokenFuse gateway's LLM proxy and the copilot's own inference spend is metered
  and capped by the Breaker exactly like any other agent — the thesis, dogfooded. Harmless
  against a raw Ollama/Anthropic endpoint.

### W1 tests
41 crate unit tests + `no_signer` guard: propose tools emit the right `ProposedAction`
(kind/target/params) and validate args (a bad verdict / missing cap is `BadArgs`); the loop
collects a proposal into `Answer.proposals`; the registry offers propose tools even with no
planes; `ProposedAction` round-trips and stays backward-decodable. All gates green
(`cargo fmt/clippy/test --workspace`, 25 suites).

## W2 — both shells (two Sonnet tracks, orchestrator-reviewed)

Both render `Answer.proposals` as cards (verb + target + params + rationale + confidence chip
+ evidence ids + a "governed by policy …" line) with **Approve** / **Dismiss**, and route
Approve into the **EXISTING human-signed ceremony** — verified by `file:line`, no signer
added, none weakened:

| kind | Tauri path | SwiftUI path |
|---|---|---|
| kill | `killRun` → `money_kill_run` (break-glass modal) | `CloudModel.killRun` (Touch ID + `BreakGlassPanel`) |
| budget | `setBudget` → `money_set_budget` (break-glass) | `CloudModel.setBudget` (Touch ID) |
| grant_deny | `decideApproval` → `policy_decide_approval` | `PolicyModel.decide` (Touch ID) |
| rescan | `rescan` → `identity_rescan` | `IdentityModel.rescan` |

**Audit link**: on a successful approve, a distinct `console.copilot_proposal_approved`
(`{kind, target}`, `decision:"allow"`) is journaled through the same already-paired handle,
alongside the mutation's own journal line — so the record reads "human approved copilot
proposal", never "copilot did it". (Two honest caveats, both flagged by the tracks:
`ProposedAction` has no `proposal_id`, so the link carries `kind`+`target`, not the doc's
illustrative id; and Idryx has no journal mechanism at all by its own design, so the Rescan
approve records a transcript note instead of forcing an inconsistent per-copilot journal.)

## Exit gate (C2)

All gates green, re-run by the orchestrator on the integrated tree: `cargo fmt --check`,
`cargo clippy --workspace --all-targets -D warnings`, `cargo test --workspace` (25 suites),
Tauri `tsc` + `pnpm build` + `src-tauri cargo build`, SwiftUI `build-ffi.sh` + `swift build`.
The propose→proposals path and the no-signer invariant are proven deterministically; the
approve routing is a verified reuse of the existing signed ceremony in both shells.

The copilot still defaults to `provider = "none"` on this box (no local model), so proposals
only appear once a provider is configured; the whole propose→confirm→audit chain is wired and
tested. A live LLM demo needs a local Ollama or a BYO cloud key (a spend opt-in).

## Deferred (post-C2)
- The precise Wardryx PDP dry-run verdict (needs a Wardryx dry mode).
- A `proposal_id` on `ProposedAction` for a first-class audit link (today `kind`+`target`).
- The false-alarm memory write-back (from C1).
- **C3** — the relay triage tie (a triage stage inside `genaryx-relay` that annotates the D12
  pager, HARD events always pushing), unchanged from PHASE6.md.
