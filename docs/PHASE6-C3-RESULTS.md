# Phase 6 · C3 (Felyx the intelligent pager — the D12 tie) — RESULTS

Status: **DONE, all gates green**, 2026-07-19. The LAST Felyx cut. Contract:
[PHASE6-C3.md](PHASE6-C3.md). After C3 the whole D13 copilot is built (C0-C3).

C3 puts Felyx in front of the D12 relay's push path as a triage stage, honoring the one
inviolable rule that is both the safety model and the pitch:

> **An AI cannot silence the pager** (the HARD floor is deterministic code in the relay) and
> **an AI cannot press its buttons** (no signer, C0-C2). The copilot may only ADD.

All C3 work is `crates/relay` + a small `crates/copilot` addition (a headless second host for
the copilot). No desktop shells; a small optional mobile render shows the annotation.

## W1 — crate (orchestrator-owned)

### `crates/copilot`: a fast annotation
- `CopilotAnnotation { summary, recommended_action: Option<ProposedAction>, confidence, chain }`
  (Serialize) — the D13.4 `copilot` block.
- `CopilotService::annotate(event)` / `Felyx::annotate(event)`: a FAST, single-turn,
  **tool-free** provider call summarizing one event in a sentence; `Ok(None)` when disabled.
  Deliberately not the tool loop — an annotation must fit the ~3 s budget.

### `crates/relay`: the triage stage (now depends on `genaryx-copilot`)
- `ExceptionItem` gained `copilot: Option<CopilotAnnotation>` (`skip_serializing_if`), and
  `ExceptionEngine::annotate_item(key, ann)` enriches a queued item. Enriching the snapshot
  IS the sim delivery path (the phone polls it).
- New `triage.rs` — `Triage`:
  - **HARD intent**: `dispatch_push` IMMEDIATELY and unconditionally (the floor), BEFORE any
    copilot call. Then, if a copilot is configured, **spawn** a budgeted task
    (`tokio::time::timeout(annotation_budget, annotate)`) that calls `engine.annotate_item`
    on success — so it never blocks the push, never delays the loop, and can never suppress.
  - **SOFT intent**: held in a soft-queue, not paged immediately.
  - `flush_soft`: drains the queue into ONE digest push, on a cadence (batch / a long
    interval = "hold to a morning summary").
- `run_event_loop` routes every `PushIntent` through `Triage::on_intent` and ticks
  `flush_soft` on `soft_flush_secs`.
- Config via env (`GENARYX_RELAY_COPILOT_*`, `_ANNOTATION_BUDGET_MS`, `_SOFT_FLUSH_SECS`);
  **local-only by default** (the residency / trial posture; a trial license hard-locks it).
  The copilot is OPTIONAL: with no provider, the relay pages exactly as it did before C3 —
  the deterministic pager always works without any AI (Q7).

### W1 tests (deterministic, no LLM)
- Relay: 59 tests (5 new). The **floor** — a HARD intent is dispatched, never parked in the
  soft-queue, and works with `copilot = None`; **SOFT batching** — a SOFT intent waits and
  `flush_soft` emits one digest (empty flush is a no-op); the digest summary; `item_key`
  matching the engine's `run:`/`incident:` format; and `annotate_item` enriching a HARD
  item's snapshot (and no-op on a missing key).
- Copilot: 43 tests (2 new) — `annotate` produces a one-line summary from a single tool-free
  turn (MockProvider), and a disabled service yields `None`.
- The **no-suppression** guarantee is structural: `on_intent` calls `dispatch_push` before it
  ever touches the copilot, and the copilot holds no signer.

## W2 — mobile render (light)
`tokenfuse-mobile`: `ExceptionItem` gains an optional `copilot` annotation (decodes to nil
when the relay omits it); `ExceptionQueueView` renders the summary (+ any recommended action)
as a distinct "Felyx" line on the card — the visible intelligent-pager payoff.

## Exit gate (C3)
All gates green, re-run by the orchestrator: `cargo fmt --check`, `cargo clippy --workspace
--all-targets -D warnings`, `cargo test --workspace` (25 suites; a transient hiccup in a
skip-graceful live test cleared on re-run — the C3 triage tests are fully deterministic).

The relay still defaults to `copilot = none` on this box (no local model), so it pages plain
— the honest state; the annotation lights up the moment a provider is configured, and the
deterministic floor + soft digest work regardless.

## Deferred (refinements)
- Late-attach: an annotation that resolves AFTER the budget still updating the snapshot
  (today dropped on timeout; the floor push already went out).
- A follow-up ENRICHED APNs push once real APNs lands (R1); today enrichment is snapshot-only.
- `recommended_action` populated in the fast annotate path (today `None`).

## Phase 6 (D13 Felyx) is now complete: C0 → C3
Read (C0) → triage + explain (C1) → propose-and-confirm (C2) → the intelligent pager (C3).
On Yurii's go, `phase-6-complete` can tag the finished copilot.
