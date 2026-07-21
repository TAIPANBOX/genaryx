#!/usr/bin/env python3
# Genaryx live-validation fleet generator v3 - enterprise tier-1-bank money plane.
#
# What changed from v2, and why it had to change
# ----------------------------------------------
# v2 ingested call records and NOTHING ELSE. It never set a single per-run
# budget. `Store::alerts` (tokenfuse cloud store.rs) iterates the org's BUDGETS
# and returns empty the moment there are none:
#
#     let Some(budgets) = inner.budgets.get(org) else { return out };
#
# and the relay builds its exception queue from `/v1/alerts`
# (genaryx crates/relay/src/exceptions.rs). So a v2 dataset gives a phone and a
# watch that say "All clear" while the console shows thousands of dollars burnt.
# That is exactly the console/phone mismatch this campaign exists to remove, and
# in the 2026-07-19 run it was papered over with a SEPARATE small mobile seed,
# which is how the two surfaces ended up quoting different numbers.
#
# v3 therefore does three things v2 did not:
#   1. names the hero generation -002 and gives it a single named protagonist
#      run, `reconciliation-batch-eod-002-LIVE`, the one every screenshot
#      follows (NEXT-CAMPAIGN.md section 5);
#   2. sets real per-run budgets through `POST /v1/runs/{run}/budget`;
#   3. sets them AFTER ingest, computed from each run's ACTUAL accumulated
#      spend, so the resulting spent/budget fraction is a chosen number rather
#      than a lucky one, and the exception queue is short, readable and
#      predictable instead of thousands of rows long.
#
# Grounded in tokenfuse cloud store.rs semantics:
#   spent_microusd   = sum cost over NON-blocked rows (allow; cache_hit cost=0)
#   blocked_spend_us = sum cost over BUDGET-PROTECTION reasons only
#                      (budget_exceeded, loop_detected, policy_violation, wasm_policy, killed)
#   cache_saved_us   = sum saved_microusd on cache_hit rows
#   router_saved_us  = sum saved_microusd on allow rows
# dlp_blocked / taint_blocked carry cost 0 (security, not dollars) - excluded from $ savings.
import json, random, time, urllib.request, urllib.error

random.seed(20260720)
BASE = "http://127.0.0.1:8080"
CLOUD = f"{BASE}/v1/ingest"
BEARER = "devkey"          # admin key: ingest + budgets. The RELAY uses relayviewer.
ORG = "meridian.example"
now = int(time.time() * 1000); HOURS = 10 * 3600 * 1000; start = now - HOURS
BUDGET_PROT = {"budget_exceeded", "loop_detected", "policy_violation", "wasm_policy", "killed"}

# The protagonist. Every surface in the capture set follows this one id.
HERO_RUN = "reconciliation-batch-eod-002-LIVE"
HERO_AGENT = f"agent://{ORG}/treasury/reconciliation-batch"

# team, name, model, runs, calls(lo,hi), cost_microusd(lo,hi), cache_rate, router_rate
FLEET = [
 ("fraud",      "fraud-triage-copilot", "claude-sonnet-5",  500, (4, 8),  (70000, 140000), 0.12, 0.18),
 ("fraud",      "txn-anomaly-scorer",   "claude-haiku-4-5", 1500,(1, 3),  (3000, 9000),    0.30, 0.10),
 ("kyc-aml",    "kyc-intake-agent",     "claude-sonnet-5",  500, (3, 6),  (60000, 130000), 0.10, 0.15),
 ("kyc-aml",    "sanctions-screener",   "claude-haiku-4-5", 1200,(1, 2),  (2500, 7000),    0.35, 0.08),
 ("kyc-aml",    "aml-case-copilot",     "gpt-4o",           400, (6, 10), (280000, 560000),0.06, 0.20),
 ("lending",    "underwriting-copilot", "claude-sonnet-5",  450, (6, 11), (80000, 170000), 0.08, 0.16),
 ("lending",    "doc-intake-ocr",       "claude-haiku-4-5", 700, (1, 3),  (4000, 14000),   0.25, 0.10),
 ("lending",    "collateral-valuator",  "gpt-4o-mini",      300, (2, 4),  (9000, 30000),   0.15, 0.22),
 ("support",    "support-tier1-bot",    "claude-haiku-4-5", 1500,(1, 2),  (3000, 8000),    0.45, 0.06),
 ("support",    "support-tier2-bot",    "claude-sonnet-5",  600, (2, 5),  (90000, 160000), 0.20, 0.14),
 ("support",    "escalation-router",    "claude-haiku-4-5", 500, (1, 1),  (2000, 5000),    0.30, 0.05),
 ("treasury",   "cashflow-forecaster",  "gpt-4o",           250, (7, 12), (300000, 600000),0.05, 0.18),
 ("treasury",   "spend-optimizer",      "claude-sonnet-5",  160, (3, 6),  (70000, 140000), 0.10, 0.30),
 ("compliance", "model-risk-validator", "gpt-4o",           180, (6, 10), (280000, 560000),0.06, 0.16),
 ("compliance", "control-tester",       "claude-haiku-4-5", 220, (2, 4),  (4000, 13000),   0.12, 0.10),
 ("compliance", "evidence-assembler",   "claude-sonnet-5",  130, (4, 8),  (80000, 160000), 0.10, 0.12),
]
IN_TOK  = {"claude-haiku-4-5": (600, 4000), "claude-sonnet-5": (4000, 60000),
           "gpt-4o": (40000, 220000), "gpt-4o-mini": (2000, 18000)}
OUT_TOK = {"claude-haiku-4-5": (150, 900), "claude-sonnet-5": (400, 3000),
           "gpt-4o": (800, 5000), "gpt-4o-mini": (300, 1500)}

recs = []
def add(run, ag, model, dec, cin, cout, cost, saved, step, ts):
    recs.append({"ts_millis": ts, "run_id": run, "model": model, "decision": dec,
                 "input_tokens": cin, "output_tokens": cout, "cost_microusd": cost,
                 "saved_microusd": saved, "step": step, "agent_id": ag})

# --- steady-state fleet -----------------------------------------------------
for team, name, model, nr, cpr, crng, cache, router in FLEET:
    ag = f"agent://{ORG}/{team}/{name}"; itk = IN_TOK[model]; otk = OUT_TOK[model]
    for r in range(nr):
        run = f"{name}-{r:04d}"; nc = random.randint(*cpr); ts0 = start + random.randint(0, HOURS - 1)
        for s in range(nc):
            ts = min(now - 1, ts0 + s * random.randint(200, 5000))
            cin = random.randint(*itk); cout = random.randint(*otk); cost = random.randint(*crng)
            if random.random() < cache:
                add(run, ag, model, "cache_hit", cin, cout, 0, cost, s, ts)
            else:
                saved = int(cost * random.uniform(0.20, 0.50)) if random.random() < router else 0
                add(run, ag, model, "allow", cin, cout, cost, saved, s, ts)

# --- scattered governance blocks (realism) ---------------------------------
uw = f"agent://{ORG}/lending/underwriting-copilot"
for i in range(6):
    add(f"underwriting-copilot-pol-{i:02d}", uw, "claude-sonnet-5", "policy_violation",
        2100, 240, 90000, 0, 0, start + random.randint(0, HOURS))
for team, name, model in [("support", "support-tier2-bot", "claude-sonnet-5"),
                          ("kyc-aml", "kyc-intake-agent", "claude-sonnet-5"),
                          ("compliance", "control-tester", "claude-haiku-4-5")]:
    ag = f"agent://{ORG}/{team}/{name}"
    for r in range(8):
        run = f"{name}-loop-{r:02d}"; ts0 = start + random.randint(0, HOURS)
        for s in range(random.randint(4, 9)):
            add(run, ag, model, "loop_detected", 5000, 400, random.randint(60000, 130000), 0, s, ts0 + s * 1500)
sup = f"agent://{ORG}/support/support-tier1-bot"
for i in range(12):
    add(f"support-tier1-bot-dlp-{i:02d}", sup, "claude-haiku-4-5", "dlp_blocked", 1400, 320, 0, 0, 0, start + random.randint(0, HOURS))
fr = f"agent://{ORG}/fraud/fraud-triage-copilot"
for i in range(5):
    add(f"fraud-triage-copilot-taint-{i:02d}", fr, "claude-sonnet-5", "taint_blocked", 3200, 180, 0, 0, 0, start + random.randint(0, HOURS))

# --- HERO INCIDENT: end-of-day reconciliation batch retry storm ------------
# The protagonist run is the batch's own control run: it is the id that appears
# on the console, the phone, the watch and in Felyx's answer. The 150 shards are
# its fan-out, and they are what makes the volume tell (thousands of calls
# against very little settled spend, because governance stopped them).
tsb = now - 80 * 60 * 1000; win = 30 * 60 * 1000

# The protagonist itself: real settled spend, then a wall of budget_exceeded as
# the per-run ceiling trips over and over.
add(HERO_RUN, HERO_AGENT, "gpt-4o", "allow", 210000, 3400, 620000, 0, 0, tsb)
for s in range(1, 78):
    add(HERO_RUN, HERO_AGENT, "gpt-4o", "allow" if s < 12 else "budget_exceeded",
        random.randint(190000, 230000), random.randint(3000, 4200),
        random.randint(520000, 640000), 0, s,
        min(now - 1, tsb + s * random.randint(4000, 12000)))

SHARDS = 150
for sh in range(SHARDS):
    run = f"reconciliation-batch-eod-002-s{sh:03d}"; ts0 = tsb + random.randint(0, win)
    add(run, HERO_AGENT, "gpt-4o", "allow", 190000, 3200, random.randint(500000, 640000), 0, 0, ts0)
    for s in range(1, 1 + 28):
        add(run, HERO_AGENT, "gpt-4o", "budget_exceeded", 210000, 3600, random.randint(500000, 620000),
            0, s, min(now - 1, ts0 + s * random.randint(1500, 6000)))

# --- projection (exact, from built records) --------------------------------
spent = sum(r["cost_microusd"] for r in recs if r["decision"] in ("allow", "cache_hit"))
blocked = sum(r["cost_microusd"] for r in recs if r["decision"] in BUDGET_PROT)
cache_s = sum(r["saved_microusd"] for r in recs if r["decision"] == "cache_hit")
router_s = sum(r["saved_microusd"] for r in recs if r["decision"] == "allow")
runs = len({r["run_id"] for r in recs})
byreason = {}
for r in recs:
    byreason[r["decision"]] = byreason.get(r["decision"], 0) + 1
print(f"records={len(recs)} runs={runs} agents={len(FLEET)+1}")
print(f"PROJECTED  spent=${spent/1e6:,.0f}  blocked/prevented=${blocked/1e6:,.0f}  "
      f"cache_saved=${cache_s/1e6:,.0f}  router_saved=${router_s/1e6:,.0f}  "
      f"total_saved=${(blocked+cache_s+router_s)/1e6:,.0f}")
print("decisions:", {k: byreason[k] for k in sorted(byreason)})

def api(path, payload=None, method="GET"):
    data = json.dumps(payload).encode() if payload is not None else None
    req = urllib.request.Request(f"{BASE}{path}", data=data, method=method,
        headers={"Authorization": f"Bearer {BEARER}", "Content-Type": "application/json"})
    return json.loads(urllib.request.urlopen(req, timeout=60).read() or b"null")

def post_batch(batch):
    api("/v1/ingest", {"records": batch}, "POST")

random.shuffle(recs); tot = 0
for i in range(0, len(recs), 1000):
    post_batch(recs[i:i + 1000]); tot += len(recs[i:i + 1000])
print(f"ingested {tot} records")

# --- budgets: set from ACTUAL spend so every fraction is a chosen number -----
#
# The exception queue must be short and legible on a 40mm watch face, so only a
# deliberately small set of runs carries a per-run ceiling. Everything else is
# ungoverned-by-budget and therefore invisible to /v1/alerts, which is the
# correct behaviour, not an omission: a budget is something an operator sets.
#
# alert_pct on the cloud (and the relay) is 0.8, so:
#   fraction >= 1.0  -> over cap   (hard, pushes unfiltered, killable)
#   0.8 <= f < 1.0   -> near cap   (soft)
#   f < 0.8          -> governed, not an exception
print("reading back per-run spend...")
run_rows = api("/v1/runs")
spent_by_run = {r["run_id"]: r["spent_microusd"] for r in run_rows}

def budget_for(run_id, target_fraction):
    """A ceiling that puts this run at (about) target_fraction of its budget."""
    s = spent_by_run.get(run_id, 0)
    if s <= 0:
        return None
    return max(1, int(s / target_fraction))

PLAN = []
# The protagonist, comfortably over its ceiling: this is the run that gets killed.
PLAN.append((HERO_RUN, 1.24))
# Three of its shards over cap too, so the fan-out is visible without the queue
# turning into 150 near-identical rows.
for sh in (7, 63, 128):
    PLAN.append((f"reconciliation-batch-eod-002-s{sh:03d}", round(random.uniform(1.05, 1.45), 3)))
# A handful of ordinary runs from other teams approaching their ceilings, so the
# queue shows a real spread of pressure rather than one lonely incident.
NEAR = [("aml-case-copilot", 0.94), ("cashflow-forecaster", 0.88),
        ("underwriting-copilot", 0.83), ("model-risk-validator", 0.91),
        ("support-tier2-bot", 0.86)]
for name, frac in NEAR:
    candidates = sorted((rid for rid in spent_by_run if rid.startswith(f"{name}-")),
                        key=lambda rid: -spent_by_run[rid])
    if candidates:
        PLAN.append((candidates[0], frac))
# And a set of well-governed runs sitting comfortably inside their ceilings.
# They prove budgets are a normal operating control, not something that only
# appears when things go wrong. They must NOT show up in /v1/alerts.
quiet = sorted((rid for rid in spent_by_run
                if not rid.startswith("reconciliation-batch")), key=lambda r: -spent_by_run[r])
for rid in quiet[:40]:
    if any(rid == p[0] for p in PLAN):
        continue
    PLAN.append((rid, round(random.uniform(0.15, 0.55), 3)))
    if len([p for p in PLAN if p[1] < 0.8]) >= 18:
        break

applied = 0
for run_id, frac in PLAN:
    micros = budget_for(run_id, frac)
    if micros is None:
        print(f"  SKIP {run_id}: no settled spend to size a budget from")
        continue
    api(f"/v1/runs/{run_id}/budget", {"budget_usd": micros / 1e6}, "POST")
    applied += 1
print(f"budgets set on {applied} runs "
      f"({len([p for p in PLAN if p[1] >= 1.0])} over cap, "
      f"{len([p for p in PLAN if 0.8 <= p[1] < 1.0])} near cap, "
      f"{len([p for p in PLAN if p[1] < 0.8])} comfortably inside)")

# --- verify the exception surface is what we intended ------------------------
alerts = api("/v1/alerts")
alerts.sort(key=lambda a: -a["fraction"])
print(f"\n/v1/alerts now returns {len(alerts)} rows (this is what the phone and watch will show):")
for a in alerts:
    band = "OVER CAP " if a["fraction"] >= 1.0 else "near cap "
    print(f"  {band} {a['fraction']:.2f}  ${a['spent_microusd']/1e6:>8,.2f} / "
          f"${a['budget_micros']/1e6:>8,.2f}  {a['run_id']}")
if not any(a["run_id"] == HERO_RUN for a in alerts):
    print(f"\n  !! {HERO_RUN} is NOT in the alert set. Every screenshot depends on it. "
          f"Do not proceed to capture.")
