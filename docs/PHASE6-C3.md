# Phase 6 · C3 — Felyx the intelligent pager (the D12 tie)

Build contract for C3, the LAST Felyx cut. Architecture: itrat-console/13 D13.4 + D13.7 C3.
After C3 the whole D13 copilot is done and `phase-6-complete` is taggable.

C3 puts Felyx in front of the D12 relay's push path as a **triage stage**, with one
inviolable rule that is the safety model AND the sales pitch:

> **An AI cannot silence the pager** (the HARD floor is deterministic code in the relay, not a
> prompt) and **an AI cannot press the pager's buttons** (no signer, C0-C2). The copilot may
> only ADD.

All C3 work is in `crates/relay` + a small `crates/copilot` addition (a headless second host
for the copilot). No desktop shells. A small optional mobile render shows the annotation.

## C3-W1 — crate (orchestrator-owned)

### `crates/copilot`: a fast annotation
- `CopilotAnnotation { summary: String, recommended_action: Option<ProposedAction>, confidence: f32, chain: Vec<String> }`
  (Serialize) — the D13.4 `copilot` block.
- `CopilotService::annotate(event: &str) -> Result<Option<CopilotAnnotation>, CopilotError>`:
  a FAST, single-turn, **tool-free** provider call that summarizes one event in a sentence.
  `Ok(None)` when the copilot is disabled. Deliberately not the full `explain_incident` loop
  — an annotation must fit a ~3 s budget, so it is one `chat` turn, no tools.

### `crates/relay`: the triage stage (depends on `genaryx-copilot`)
- `ExceptionItem` gains `copilot: Option<CopilotAnnotation>` (`#[serde(default)]`); the phone
  polls this, so enriching the snapshot IS the sim delivery path.
- `ExceptionEngine::annotate_item(key, ann)` stores an annotation on a queued item.
- `triage.rs` — `Triage { copilot: Option<Arc<CopilotService>>, soft_queue, config }`:
  - **HARD intent** (`intent.hard`, i.e. over_cap/runaway): `dispatch_push` IMMEDIATELY and
    unconditionally (the floor). Then, if the copilot is enabled, **spawn** a budgeted task
    (`tokio::time::timeout(annotation_budget, copilot.annotate(desc))`) that calls
    `engine.annotate_item` on success — so it never blocks the push, never delays the loop,
    and can never suppress. On timeout/disabled, the item stays plain (already delivered).
  - **SOFT intent**: NOT pushed immediately — enqueued in the soft-queue.
  - `flush_soft`: drains the queue and emits ONE digest push ("7 warnings near cap: …"),
    called on `soft_flush_interval` (a long interval = the "morning digest"/hold mode).
- `run_event_loop` routes every `PushIntent` through `Triage` instead of calling
  `dispatch_push` directly, and ticks `flush_soft` on its interval.

### Config (relay `config.rs`)
A `[copilot]` block (reuse `genaryx_copilot::CopilotConfig`) + triage knobs:
`annotation_budget_ms` (default 3000), `soft_mode` = `immediate | batch | hold`,
`soft_flush_secs`. **Trial-mode lock**: when the license is a trial, force
`allow_non_local_endpoints = false` (local providers only — the strongest residency demo).
Copilot is OPTIONAL: absent provider ⇒ the relay behaves exactly as D12/C-pre-3 (plain
pushes), so the deterministic pager always works without any AI (Q7).

### Tests (deterministic, no LLM)
- **Floor**: a HARD intent is dispatched even with `copilot = None`, and even when the
  annotation times out (a slow test provider) — the push is never blocked or dropped.
- **Annotation**: with a MockProvider, a HARD item gets `copilot.summary` set on the snapshot.
- **No suppression**: the copilot cannot stop a HARD push (structural — dispatch precedes any
  annotate call).
- **SOFT**: a SOFT intent is not pushed immediately; `flush_soft` emits one digest.

## C3-W2 — mobile render (light, optional)
`tokenfuse-mobile`: `ExceptionItem` gains the optional `copilot` field (serde default);
`ExceptionQueueView` renders `item.copilot.summary` (+ the recommended action if present) on
the card — the visible "intelligent pager" payoff. Skip-graceful when absent.

## Deferred (post-C3, refinements)
- Late-attach: an annotation that resolves AFTER the budget still updating the snapshot (today
  it is dropped on timeout; the floor push already went out).
- A follow-up ENRICHED APNs push once real APNs lands (R1); today enrichment is snapshot-only
  (the sim poll path).
- `recommended_action` populated in the fast annotate path (today `None`; the full proposal
  path is C2's `explain`/propose tools).

## Gates (unchanged)
`cargo fmt --check`, `cargo clippy --workspace --all-targets -D warnings`,
`cargo test --workspace`; mobile `swift build`/`xcodebuild` if W2 is done.
