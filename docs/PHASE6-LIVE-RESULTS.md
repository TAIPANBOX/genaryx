# Phase 6 (D13 Felyx) - live validation with a real cloud model

This is the live counterpart to the deterministic `PHASE6-C{0,1,2,3}-RESULTS.md`
gates. Those prove the copilot with a `MockProvider` and a fake server on the
build machine (no network, fully reproducible). This run proves the SAME code
path end to end against a REAL inference provider and a REAL money plane holding
a realistic fleet, so nothing about the live seam is left to assumption.

- **Date:** 2026-07-19
- **Provider:** Anthropic `claude-sonnet-5` (BYO-cloud), `endpoint=https://api.anthropic.com`, `local=false`
- **Money plane:** `tokenfuse-cloud` seeded with the `meridian.example` fleet
- **Host:** an ephemeral 16 vCPU / 32 GB Hetzner box, torn down after the run

The copilot ships in paid Genaryx and defaults to a LOCAL provider (residency).
Here we deliberately exercise the BYO-cloud path (the residency gate requires the
explicit `allow_non_local_endpoints` opt-in, set for the demo) so the cloud seam
is covered too. The deterministic pager (D12) needs no AI at all; the AI only
ever ADDS.

## Environment

| Piece | Value |
|---|---|
| Box | Ubuntu, 16 vCPU / 32 GB, `ufw` locked to SSH only (Cloud + relay bind loopback / are firewalled off the internet) |
| Cloud seed | `9,287 runs`, `34,824 calls`, `$4,314.42 spent`, `176 open incidents` (org `meridian.example`) |
| Cloud keys | `devkey` (admin, seed + copilot reads), `relaykey` (viewer, relay) |
| Copilot key | Anthropic key delivered to the box as a `0600` file, referenced via `file:` (never echoed to a log or committed) |

## How it was driven

- C0 / C1 / C2: the `live_felyx_demo` ignored test in `crates/copilot/src/service.rs`, built the real `CloudClient`, and ran three prompts against `claude-sonnet-5`:
  ```
  GENARYX_COPILOT_PROVIDER=anthropic GENARYX_COPILOT_MODEL=claude-sonnet-5 \
  GENARYX_COPILOT_API_KEY_REF=file:/root/felyx-anthropic-key GENARYX_COPILOT_ALLOW_REMOTE=1 \
  GENARYX_DEMO_CLOUD_URL=http://127.0.0.1:8080 GENARYX_DEMO_CLOUD_KEY=devkey \
  cargo test -p genaryx-copilot live_felyx_demo -- --ignored --nocapture
  ```
- C3: the real `genaryx-relay` binary, subscribed to the Cloud SSE, with the copilot enabled via `GENARYX_RELAY_COPILOT_*`. A fresh runaway run was then ingested to trigger a live HARD event.

Only the Cloud plane was configured on this box, so Idryx / Wardryx / Engram tools
were intentionally absent. That is a feature of the run: it shows how Felyx
degrades (it names the gap instead of inventing identity / policy / memory data).

## C0 - money Q&A (tool-computed numbers, not prose arithmetic)

Prompt: *"Which agents or runs have blown their budget or look runaway ... Check `alerts` and `incidents`, and use `list_runs` for the biggest spenders. Be brief and cite specific ids."*

- Tools the model actually called: `["alerts", "incidents", "list_runs"]`
- Usage: 13,413 + 989 tokens

It found the two `fanout_explosion` incidents (`support-tier1-bot` at 1,019
occurrences, `txn-anomaly-scorer` at 1,103), identified the `reconciliation-batch`
family as the dominant `budget_exhausted` pattern (176 incidents), listed the top
spenders (`cashflow-forecaster-0217` at $5.85, down to ~$5.23 for the 30th), and
cited the `$4,314.42` org total. It was honest that `alerts` was empty ("no runs
currently sitting at/above their live budget-alert threshold"). Every number came
from a tool call, none from memory.

## C1 - explain_incident (cross-plane root cause, honest about gaps)

Called `explain_incident("budget_exhausted:kyc-intake-agent-loop-00")`.

- Tools called: `["incidents", "alerts", "list_runs"]`
- Usage: 13,848 + 1,746 tokens

The model opened with a scope correction rather than a fabrication:

> I don't have `identity_alerts`, `policies`, or `memory_recall` tools in my actual toolset ... I pulled what I could and am flagging the rest as gaps rather than inventing data.

Then it gave a money-plane root-cause chain (undersized budget cap -> 5
`budget_exhausted` hits inside a 6-second window -> high-severity, unacknowledged
incident), and explicitly refused to assert identity / policy / precedent it
could not verify. This is exactly the intended behavior when a plane is absent.

## C2 - propose (recommend, never act)

Prompt: *"... which single run or agent would you recommend killing to stop the bleed, and why? Propose it with evidence."*

- Tools called: `["list_runs", "alerts", "incidents", "list_agents", "money_summary", "savings", "propose_kill"]`
- Usage: 51,793 + 5,412 tokens
- Proposal emitted (the shell would render this as an approve / sign card):

```
Kill target=cashflow-forecaster-0217 confidence=0.65
rationale: Top single-run spender fleet-wide ($5.85, 5,853,915 µUSD) with 0 cache
  hits across 12 calls. It's representative of a systemic pattern: all 30
  highest-spending runs in the org belong to this one agent
  (treasury/cashflow-forecaster), each costing ~$5.2-5.9 with essentially no
  caching despite ~250 near-identical runs ($1,038.95 aggregate spend, #2 agent
  org-wide). No budget alert has tripped because each run stays just under
  threshold, letting the spend bleed out quietly across many runs rather than one
  obvious runaway.
evidence: ["list_runs:cashflow-forecaster-0217", "list_runs:cashflow-forecaster-0211",
  "list_runs:cashflow-forecaster-0206",
  "list_agents:agent://meridian.example/treasury/cashflow-forecaster"]
```

The insight is the real FinOps value: a distributed, under-threshold spend bleed
that per-run alerting never catches. And the boundary held. The model closed
with: *"I did not act on anything; this is a recommendation pending your
sign-off."* Felyx holds no signer (structural, C0-C2), so the proposal is a
descriptor only; a human must approve and the existing device-signed ceremony
performs any real kill.

## C3 - the relay annotates a HARD event live (D12 tie)

The `genaryx-relay` started with the copilot enabled:

```
genaryx-relay: copilot annotation enabled (anthropic / claude-sonnet-5, local=false)
```

A fresh runaway run (`reconciliation-batch-eod-002-LIVE`, 8 `loop_detected` + 7
`budget_exceeded` records) was ingested. The Cloud broadcast the incidents; the
relay triaged them. The ordering in the log is the whole point:

```
05:06:48  would push (no APNs token on file): ... reconciliation-batch-eod-002-LIVE running hot - budget_exhausted. Tap to review and kill.
05:06:48  would push (no APNs token on file): ... reconciliation-batch-eod-002-LIVE running hot - sustained_loop. Tap to review and kill.
05:06:50  triage: attached copilot annotation to run:reconciliation-batch-eod-002-LIVE: reconciliation-batch-eod-002-LIVE is stuck in a sustained loop and burning resources unattended—review and kill it now to prevent runaway costs or duplicate/corrupt EOD reconciliation output.
05:06:50  triage: attached copilot annotation to run:reconciliation-batch-eod-002-LIVE: Reconciliation batch job eod-002-LIVE has exhausted its budget and is stuck running hot—needs immediate review and termination to prevent runaway resource use or cost overrun.
```

- The deterministic HARD floor dispatched at `05:06:48`, BEFORE any model call. An AI can never silence the pager.
- The budgeted annotation attached ~2 seconds later, inside its latency budget, and only ENRICHED the already-delivered push. It even inferred the domain risk ("duplicate/corrupt EOD reconciliation output") from the run name alone.
- ("would push" is the `NullSender` speaking: no phone was paired this round. Real APNs delivery needs an Apple Developer account and is out of scope here.)

## Two robustness gaps the live run surfaced (and the fixes in this commit)

Deterministic gates with small fixtures never hit these; a real 9k-run fleet did.

1. **`list_runs` returned the entire fleet.** `/v1/runs` has no server-side limit,
   so the tool serialized all 9,287 runs (~2.15 MB, ~1.08M tokens) into the prompt
   and the provider rejected it with HTTP 400 (over the 1M context limit). A tool
   must never hand the model unbounded data. Fix (`crates/copilot/src/tools/cloud.rs`):
   `list_runs` now returns the top rows by spend plus the true org-wide count and
   total, and `incidents` is capped the same way. This is also strictly better for
   the task (the top spenders are exactly what a kill / budget question is about).

2. **A truncated final turn produced a blank answer.** When the model's last turn
   hit `max_tokens` mid-synthesis, the loop returned empty text. Fix: the demo runs
   with `max_tokens = 4096` headroom so the final answer is never cut. (The shipped
   default stays conservative; this is a demo-config value. A follow-up could make
   the loop surface a truncation explicitly rather than returning empty.)

A third, minor change: the relay's `annotate_hard` now logs the annotation it
attached (symmetry with the failure arms, which already logged), which is what
made the C3 result observable above.

## What was NOT done

- **Real APNs push to a physical phone.** Needs an Apple Developer account (paid,
  user-held). The relay's HARD floor + budgeted annotation are fully proven; only
  the last-hop delivery to a real device is deferred.

## Reproduce

Bring up `tokenfuse-cloud` with `TOKENFUSE_CLOUD_KEYS="devkey:<org>:admin:paid,relaykey:<org>:viewer:paid"`,
seed it (`live-campaign/scripts/gx_fleet_v2.py`), then run the `live_felyx_demo`
command above for C0-C2 and the `genaryx-relay` with `GENARYX_RELAY_COPILOT_*` for
C3. Full transcripts were captured to `felyx-c012.log` and `felyx-c3.log`.
