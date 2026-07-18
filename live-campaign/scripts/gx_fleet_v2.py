#!/usr/bin/env python3
# Genaryx live-validation fleet generator - enterprise tier-1-bank money plane.
# Grounded in tokenfuse cloud store.rs semantics:
#   spent_microusd   = sum cost over NON-blocked rows (allow; cache_hit cost=0)
#   blocked_spend_us = sum cost over BUDGET-PROTECTION reasons only
#                      (budget_exceeded, loop_detected, policy_violation, wasm_policy, killed)
#   cache_saved_us   = sum saved_microusd on cache_hit rows
#   router_saved_us  = sum saved_microusd on allow rows
# dlp_blocked / taint_blocked carry cost 0 (security, not dollars) - excluded from $ savings.
import json, random, time, urllib.request
random.seed(20260717)
CLOUD = "http://127.0.0.1:8080/v1/ingest"; BEARER = "devkey"; ORG = "meridian.example"
now = int(time.time() * 1000); HOURS = 10 * 3600 * 1000; start = now - HOURS
BUDGET_PROT = {"budget_exceeded", "loop_detected", "policy_violation", "wasm_policy", "killed"}

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
rag = f"agent://{ORG}/treasury/reconciliation-batch"; tsb = now - 80 * 60 * 1000; win = 30 * 60 * 1000
SHARDS = 150
for sh in range(SHARDS):
    run = f"reconciliation-batch-eod-001-s{sh:03d}"; ts0 = tsb + random.randint(0, win)
    add(run, rag, "gpt-4o", "allow", 190000, 3200, random.randint(500000, 640000), 0, 0, ts0)
    for s in range(1, 1 + 28):
        add(run, rag, "gpt-4o", "budget_exceeded", 210000, 3600, random.randint(500000, 620000),
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

def post(batch):
    b = json.dumps({"records": batch}).encode()
    req = urllib.request.Request(CLOUD, data=b, method="POST",
        headers={"Authorization": f"Bearer {BEARER}", "Content-Type": "application/json"})
    urllib.request.urlopen(req, timeout=60).read()
random.shuffle(recs); tot = 0
for i in range(0, len(recs), 1000):
    post(recs[i:i + 1000]); tot += len(recs[i:i + 1000])
print(f"ingested {tot} records")
