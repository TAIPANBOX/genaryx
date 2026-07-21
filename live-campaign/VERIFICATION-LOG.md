# Live verification log, campaign of 2026-07-20

Running record of things actually checked against the live box, so `RESULTS.md`
and the article can quote them without anyone recalling from memory. Every entry
is a command that was run and the output it produced.

Box: Hetzner CPX62 `5.75.234.176` (`ubuntu-32gb-fsn1-1`), Ubuntu 26.04, 16 vCPU / 30 GB.
Provisioned 2026-07-20 by Yurii, torn down by him afterwards.
SSH key `~/.ssh/hetzner-genaryx-20260720` (never delete on my own initiative).

---

## 1. Cloud principals: the relay's viewer key is enforced by the Cloud

**Why this exists.** The D12 trust boundary says the relay "cannot mutate anything
at the Cloud" because its key is a viewer, never an admin, key. `stack-up` starts
the cloud in devkey mode (`TOKENFUSE_CLOUD_KEYS=""` + `ALLOW_DEVKEY=1`), and that
lone `devkey` principal is `org=default, role=admin, plan=Paid`
(`tokenfuse/crates/cloud/src/keys.rs`). Running the campaign that way would have
meant the relay held an ADMIN key while the article claimed otherwise.

**What was changed.** `/root/stack-up/up.sh` patched so the cloud's key spec is
overridable from the environment instead of hardcoded empty (default behaviour
unchanged when nothing is exported; original kept at `up.sh.orig`):

```
-TOKENFUSE_CLOUD_KEYS="" \
-TOKENFUSE_CLOUD_ALLOW_DEVKEY="1" \
+TOKENFUSE_CLOUD_KEYS="${TOKENFUSE_CLOUD_KEYS:-}" \
+TOKENFUSE_CLOUD_ALLOW_DEVKEY="${TOKENFUSE_CLOUD_ALLOW_DEVKEY:-1}" \
```

Stack relaunched (`down.sh`, then `up.sh --no-demo`, so the two-run demo dataset
is NOT mixed into the campaign numbers) with:

```
TOKENFUSE_CLOUD_KEYS="devkey:default:admin:paid,relayviewer:default:viewer:paid"
TOKENFUSE_CLOUD_ALLOW_DEVKEY="0"
```

**Verified live**, three principals against the running cloud on `:8080`:

| Bearer | `GET /v1/summary` | `POST /v1/runs/probe-run/kill` | `POST /v1/pair/new` |
|---|---|---|---|
| `devkey` (admin) | 200 | 200 | 200 |
| `relayviewer` (viewer) | 200 | **403** | **403** |
| an unknown key | 401 | 401 | 401 |

**What this licenses us to say.** The relay reads the fleet and computes the
exception slice with `relayviewer`, and the Cloud itself refuses that key both
the cross-fleet kill and the minting of pairing codes. Pairing still works,
because the relay redeems a code at `POST /v1/pair`, which takes no bearer at
all (the one-time code is the credential); minting requires the admin key and
happens only on the desktop.

**What this does NOT license us to say.** A device's own kill is authorized by
that device's token, forwarded verbatim by the relay's mutation passthrough, not
by the viewer key. The viewer key proves the relay cannot act on its own; it says
nothing about what a paired phone or watch may do.

### Cleanup: the proof itself mutated state, and that state was flushed

The admin row of the table above is a REAL kill, not a dry run. `POST
/v1/runs/probe-run/kill` with `devkey` returned 200 and left a standing kill
order for a run that does not exist:

```
GET /v1/kills  ->  ["probe-run"]
```

Left alone, that would have put a kill with no matching run into the very
counter this campaign is being re-shot to fix. The cloud is in-memory, and
nothing had been seeded yet, so the stack was restarted to flush it. Confirmed
afterwards:

```
GET /v1/kills    ->  []
GET /v1/summary  ->  {"runs":0,"calls":0,"spent_microusd":0}
POST /v1/runs/x/kill as relayviewer -> 403   (still refused after the restart)
```

Rule for the rest of this campaign: **no probe writes against the campaign
dataset once seeding starts.** Anything that has to be proven by mutating gets
proven before the seed, or on a throwaway org.

---

## 2. Services and their state at seed time

`stack-up` running with `--no-demo`, so the two-run demo dataset is absent and
every number in the final dump traces to our own seeders.

Listening: `:4100` gateway, `:8080` cloud, `:8081` idryx, `:8090` wardryx.
The web dashboard on `:3000` did NOT come up; it is not used by this campaign
(the screenshots are of Genaryx, not the web dashboard), so it was left alone
rather than chased. Noted so nobody later reads its absence as a failure.

---

## 3. The money plane needed budgets, and v2 of the generator set none

**The defect that would have sunk the mobile half of the campaign.**
`gx_fleet_v2.py` ingests call records and never calls
`POST /v1/runs/{run}/budget`. `Store::alerts` (tokenfuse `crates/cloud/src/store.rs`)
opens with:

```rust
let Some(budgets) = inner.budgets.get(org) else { return out };
```

so with no budgets set, `/v1/alerts` is empty. The relay builds its exception
queue from `/v1/summary` + `/v1/alerts` + `/v1/incidents`
(`genaryx/crates/relay/src/exceptions.rs`), so the phone and the watch would
have shown "All clear" while the console showed thousands of dollars burnt.
In the 2026-07-19 run this was worked around with a SEPARATE small mobile seed,
which is precisely why the two surfaces quoted different numbers.

**`gx_fleet_v3.py`** (new, v2 kept for provenance) fixes it: hero generation
renamed to -002 with a single named protagonist run, and per-run budgets set
AFTER ingest and computed from each run's ACTUAL settled spend, so every
spent/budget fraction is a chosen number rather than a lucky one.

### Seeded fleet, read live from the cloud (`gx_verify.sh`)

> **These are POST-SEED numbers, not the final dump.** State changes still to
> come will move them: the console/phone/watch kills, and the Mockryx fire drill,
> which fires real traffic at the live gateway on `:4100` and therefore adds real
> calls and real governance decisions to this same dataset. Per the capture
> protocol (NEXT-CAMPAIGN.md section 7) the numbers that go into the article come
> from the dump taken at capture time, after every state change. Do not quote the
> table below in any deliverable.

| Metric | Raw | Value |
|---|---|---|
| Runs | `9,288` | |
| Calls | `34,678` | |
| Actual AI spend | `4,254,668,829` microUSD | **$4,254.67** |
| Prevented (budget kill-switch) | `2,404,596,227` | **$2,404.60** |
| Semantic-cache savings | `353,677,693` | **$353.68** |
| Model-router savings | `264,152,938` | **$264.15** |
| Total governed savings | `3,022,426,858` | **$3,022.43** |
| Budget breaks | | **181** |
| Incidents | | **180** (175 `budget_exhausted`, 2 `sustained_loop`, 2 `fanout_explosion`, 1 `spend_spike`) |
| Alerts (the exception surface) | | **9** |

Top agents by spend: aml-case-copilot $1,259 (3,186 calls), cashflow-forecaster
$1,006 (2,348), model-risk-validator $574 (1,443), underwriting-copilot $442
(3,835), fraud-triage-copilot $276 (3,007).

**The runaway tell**, unchanged in character from the previous campaign:
`reconciliation-batch` settled only **$93 across 4,428 calls**. Near-invisible
by money, glaring by volume, because governance stopped it before it could burn.

### The exception surface the phone and watch will show

9 rows, deliberately short enough to read on a 40mm watch face:

```
OVER CAP  1.35   $0.58 / $0.43   reconciliation-batch-eod-002-s007
OVER CAP  1.24   $6.91 / $5.57   reconciliation-batch-eod-002-LIVE   <- protagonist
OVER CAP  1.21   $0.63 / $0.52   reconciliation-batch-eod-002-s063
OVER CAP  1.16   $0.53 / $0.46   reconciliation-batch-eod-002-s128
near cap  0.94   $4.84 / $5.15   aml-case-copilot-0377
near cap  0.91   $4.81 / $5.29   model-risk-validator-0178
near cap  0.88   $5.87 / $6.67   cashflow-forecaster-0005
near cap  0.86   $0.72 / $0.83   support-tier2-bot-0361
near cap  0.83   $1.53 / $1.85   underwriting-copilot-0138
```

27 runs carry a budget in total: 4 over cap, 5 near cap, 18 sitting comfortably
inside. The quiet 18 exist so budgets read as a normal operating control rather
than something that only appears when things break; they correctly do NOT
appear in `/v1/alerts`.

---

## 4. Policy plane

6 meridian policies and 5 pending approvals, seeded live over HTTP to wardryx
on `:8090`. The treasury approval names the protagonist:

```
run_id: reconciliation-batch-eod-002-LIVE
reason: estimated cost $48.00 exceeds policy "treasury-human-approval"
        threshold $25.00; human approval required
```

`gx_policy_seed.py` had this pinned to `reconciliation-batch-eod-001-s042`, a
run id from the PREVIOUS generation, which would have put a different run on the
Policy tab than on every other surface. Fixed 2026-07-20.

---

## 5. Identity plane

`gx_idryx.py` built 785 events / 29 identities, served by a SECOND idryx
instance on `:8082` (`--load tokenfuse:/tmp/meridian-idryx.ndjson`), separate
from the stack-up idryx on `:8081`. Never kill the stack-up one: stack-up tears
down the whole stack if any child dies, which would flush the in-memory cloud.

Read live from `GET /api/identities` (note: `/api/`, not `/v1/`):
**29 identities, 43 detector alerts.**

---

## 5b. The box was open to the internet, and was closed

Found 2026-07-20 while planning where to run the relay. `ss -ltn` on the box
showed the TokenFuse Cloud bound to `0.0.0.0:8080`, not loopback, and
`ufw status` was `inactive`. Confirmed from OUTSIDE the box, from this Mac:

```
GET http://5.75.234.176:8080/v1/summary   Authorization: Bearer devkey
  -> 200 {"runs":9288,"calls":34678,"spent_microusd":4254668829}
GET the same with no bearer
  -> 401
```

`devkey` is not a secret. It is the documented dev credential in the PUBLIC
`stack-up` README, and on this box it resolves to `org=default, role=admin,
plan=Paid`. So anyone scanning Hetzner ranges had read access to the whole
fleet and, worse, `POST` on `/v1/runs/{run}/kill`, `/v1/runs/{run}/budget` and
`/v1/pair/new` (minting pairing codes).

Closed with ufw, SSH allowed first so the session could not lock itself out:

```
ufw allow 22/tcp
ufw default deny incoming
ufw default allow outgoing
ufw --force enable
```

Verified after: the cloud is unreachable from outside (connection refused /
timeout), SSH still works, and the stack answers normally over loopback on the
box. The dataset was not touched.

**Why this is in the record and not just fixed quietly.** Ф4's exit gate in the
previous campaign explicitly included "control plane closed to the internet
(ufw)". This fresh box never got that step, because provisioning went straight
from toolchains to deploying the stack. Any future run of `gx_setup.sh` should
close the box BEFORE `gx_deploy.sh` opens anything, not after. It also means the
window between deploy and this fix is a period in which the campaign dataset was
publicly readable and writable; nothing in it is real customer data (the bank is
modelled), and the numbers verified above match what was seeded, but the honest
statement is that the exposure existed rather than that it did not matter.

---

## 5c. The pager was showing 189 rows, and the same run twice

Found by pairing a device for real against the seeded fleet and diffing what
the phone sees against what the console sees, BEFORE any screenshot
(NEXT-CAMPAIGN.md section 3 asks for exactly this).

The good half first: the chain works and the data is coherent. Pairing succeeded,
the crossed-code guard refused a watch code presented as a phone code
(`400 kind_mismatch`), the SPKI pin computed from the wire OUTSIDE the box
matched what the relay printed, and aggregate spend matched the console to the
microUSD (`4254668829` both sides).

The shape was wrong:

```
GET /v1/alerts        ->   9 rows
GET /relay/v1/exceptions -> 189 rows   (9 alert-derived + 180 incident-derived)
```

and `reconciliation-batch-eod-002-LIVE`, the protagonist of every screenshot,
appeared TWICE: once from the alert ($6.91 / $5.57, over cap, fraction 1.24) and
once from an incident ($0.00 / $0.00, fraction 0.00). On screen that reads as a
broken app. Cause: `reconcile` keyed alerts as `run:<id>` and incidents as
`incident:<id>`, so the same run under both sources produced two rows.

**This was not a regression from this campaign's changes.** `reconcile` was
untouched. The defect was invisible until now because the previous campaign fed
the phone from a SEPARATE small dataset, which is the exact shortcut this re-run
exists to remove. A real fleet exposed it on first contact.

### The fix: separate what you can act on from what already happened

A run over its cap that is still spending is an ACTION, it can be killed. A
`budget_exhausted` incident is a REPORT that the breaker already tripped and the
spending already stopped. The old list conflated them, which is why 150 shards of
one batch produced 150 rows.

- Identity is the RUN. An incident about a run that already has an alert MERGES
  into it: the alert keeps the money, the incident contributes its id, kind,
  severity and timestamps, and the class is widened but never narrowed.
- `budget_exhausted` (already contained) rolls up into a `digest`, grouped by
  (kind, agent), with an exact count.
- `sustained_loop`, `spend_spike`, `fanout_explosion` are live signals, so they
  keep individual rows. An unknown kind also keeps a row: fail toward visibility.
- Truncation is now reported as `queue_truncated` instead of a silent
  `Vec::truncate`. A governance surface must not say "this is everything" when
  it is not.

### Verified live, same fleet, after the change

```
QUEUE (14 rows), truncated=0
  over_cap  budget_exhausted  1.24  $6.91  reconciliation-batch-eod-002-LIVE
  over_cap  budget_exhausted  1.35 / 1.21 / 1.16   three shards
  runaway   spend_spike, fanout_explosion x2, sustained_loop x2
  near_cap  five ordinary runs, 0.94 down to 0.83
DIGEST (1 row)
  budget_exhausted  agent=.../treasury/reconciliation-batch  count=171  severity=high
duplicate runs in queue: NONE
```

189 rows became 14 plus one counted line, and the protagonist appears once with
its real numbers. Every source row is accounted for: 9 alerts (4 of which
absorbed an incident) + 180 incidents (4 merged, 5 kept their own row, 171
digested).

Relay suite: 95 passing, clippy clean workspace-wide.

### Grouped by agent, not by kind alone

`Incident` already carries `agent_id`, so the digest groups by (kind, agent).
That is what turns an unreadable count into a sentence. Final live shape:

```
digest as the apps decode it:
  reconciliation-batch     count=147  kind=budget_exhausted
  control-tester           count=8    kind=budget_exhausted
  support-tier2-bot        count=8    kind=budget_exhausted
  kyc-intake-agent         count=8    kind=budget_exhausted

wire keys: agent_id, count, kind, last_seen_unix, severity
top level: aggregates, digest, queue, queue_truncated
```

"reconciliation-batch: 147 runs hit their ceiling" is the line that shows
governance working at scale. "147 budget_exhausted" is not.

### Rendered on both surfaces

Phone (`ExceptionQueueView`) and watch (`WatchExceptionsView`) both draw an
"ALREADY STOPPED" section below the actionable queue, deliberately quieter than
it: nothing there needs a decision. `queue_truncated`, when nonzero, gets its own
line in warning colour on both, so a cut list can never pass for a whole one.

`IncidentKind.describe` moved from `IncidentsView.swift` (phone only) into
`APIModels.swift` (both targets) rather than being duplicated, so one event
cannot end up with two different names in one app.

Both schemes build clean, no Swift diagnostics.

---

## 8. Drills plane (Mockryx), previously empty

Run for real against the LIVE gateway on `:4100` through the tunnel, using the
bundled `runaway-budget.yaml`, which rehearses this campaign's own incident in
miniature: give an agent a tiny budget, tell it to keep looping, and expect the
budget breaker to cut it off with HTTP 402 before it can run away.

```
verdict: every rehearsed guardrail HELD
scenario: runaway-budget   status: passed   calls=2   burned=$0.0028
run_id: mockryx-1784554600409496000
report: live-campaign/drills/drill-20260720T133640Z.json
```

Two calls. The breaker tripped on the second, having spent a third of a cent.
That is the same shape as `reconciliation-batch-eod-002-LIVE` hitting its
ceiling, at a scale small enough to read in one line.

**It did NOT move the campaign numbers, and that was checked rather than
assumed.** I expected the drill's traffic to land in the fleet and said so in
`gx_drills.sh`'s header. Verified afterwards:

```
mockryx agents in the cloud: 0
total agents: 17          (unchanged)
GET /v1/summary: runs 9,288  calls 34,678  spent $4,254.67   (unchanged)
```

The gateway refused both calls at its own edge with 402 and never shipped them
to the Cloud as billable records, so the seeded fleet is untouched. That is a
cleaner outcome than expected: the fleet numbers stay exactly as seeded, and the
drill stands as its own evidence in its own report, which is what Genaryx's
Drills tab reads. The caution in the script's header still holds for a drill
whose calls DO get through (a scenario the gateway allows), so it stays.

---

## 6. Quality plane (Verdryx), previously empty

Seeded by `gx_quality.py` through the real CLI, never by writing rows into the
SQLite file. 32 cases about this bank's own tasks (transaction-anomaly
judgement, sanctions-screening precision, KYC document extraction, underwriting
rationale, reconciliation-discrepancy classification), run TWICE against a real
model, then a baseline, a drift check and cost-per-correct.

Read back directly out of `~/.taipan/verdryx.db`, which is the file Genaryx
opens read-only:

| eval run | model | cases | mean | cost |
|---|---|---|---|---|
| `533e17d2` | `stub` | 5 | 0.000 | $0.0000 |
| `d69cecd9` | `claude-haiku-4-5-20251001` | 5 | 0.780 | $0.0089 |
| `dd38a38f` | `claude-haiku-4-5-20251001` | **32** | **0.886** | $0.0093 |
| `6ca04f70` | `claude-haiku-4-5-20251001` | **32** | **0.883** | $0.0091 |

The first two predate this campaign. Baselines: 2, the campaign's own is
`588ab7c1` (`meridian-quality-2026-07-20-run1`, mean 0.886).

Drift of run 2 against that baseline: 0.883 vs 0.886, delta **-0.003**,
t = -0.04 (n=32), 95% CI [-0.148, +0.137], verdict **on-track**. Unforced: the
runs were not engineered to show a regression, and they did not.

**Real Anthropic spend: $0.0184** across 66 model calls / 10,693 tokens, against
the roughly $7 available on the key.

### Disclosed: one input was authored, not exported

`verdryx cost-per-correct` was run for real, but its input file
(`gx_quality_costper.ndjson`, 14 records) was hand-authored in the second of the
two input shapes Verdryx documents, because no local Cloud was running on this
Mac to export a Parquet trace from (the campaign Cloud is on the Hetzner box).
Costs in it are grounded in `gx_fleet_v3.py`'s real ranges and reuse
`gx_policy_seed.py`'s exact figures where run ids overlap.

This does not reach any screenshot: Genaryx's Quality panel never reads
cost-per-correct output at all. It opens `verdryx.db` read-only and reads
`eval_runs`, `scores` and `baselines`, all three of which came from genuine
`eval` and `baseline` CLI runs against a real model.

---

## 7. Memory plane (Engram), previously 5 episodes and 0 facts

Seeded by `gx_memory.py` through Engram's own `observe()` / `assert_fact()`,
which is the honest shape: Genaryx deliberately exposes `stats`, `recall`, `why`
and `forget` but NOT `remember`, on the principle that agents write their own
memories and a governance console does not fabricate them. The seeder therefore
plays the agents, not the console.

```
before: episodes=5   facts=0
after:  episodes=36  facts=21     (verified by reading the SQLite directly)
```

31 episodes across 17 agents plus 21 semantic facts (team membership, model per
agent, ownership, which policy governs which team). Spot-checked that the
episodes genuinely name the protagonist, for example: "Per-run budget ceiling
tripped on reconciliation-batch-eod-002-LIVE itself: allow decisions stopped
after the..."

`recall("why did the reconciliation batch keep retrying", mode=hybrid)` returned
8 hits, top score 0.700. `why()` exercised on both an episodic id and a semantic
fact id (`reconciliation-batch governed_by_policy treasury-human-approval`), with
`extracted_by_reflection_run` and `extraction_model` correctly null because the
fact was asserted, not LLM-derived. No model calls and no cost: `observe` and
`assert_fact` never invoke an LLM.

---
