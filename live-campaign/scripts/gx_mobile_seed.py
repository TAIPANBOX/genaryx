#!/usr/bin/env python3
# Small, mobile-friendly meridian dataset for TokenFuse Pocket (the phone chokes
# on the 9k-run campaign fleet - no pagination). ~40 runs, real meridian agents,
# a mix of live / near-cap / over-cap / killed, with budgets set so the fuse bars
# read. Points at a SEPARATE cloud instance (:8083) so the main :8080 stays the
# big Genaryx campaign fleet.
import json, random, time, urllib.request
random.seed(20260718)
CLOUD = "http://127.0.0.1:8083"
BEARER = "devkey"
now = int(time.time() * 1000)

# agent, model, n_runs, per-run (cost_lo, cost_hi) microUSD, cache_rate
FLEET = [
    ("fraud/fraud-triage-copilot",       "claude-sonnet-5",  6, (60000, 130000), 0.12),
    ("kyc-aml/aml-case-copilot",         "gpt-4o",           6, (180000, 420000), 0.08),
    ("lending/underwriting-copilot",     "claude-sonnet-5",  6, (80000, 160000), 0.10),
    ("treasury/cashflow-forecaster",     "gpt-4o",           6, (150000, 340000), 0.06),
    ("support/support-tier2-bot",        "claude-sonnet-5",  6, (70000, 140000), 0.20),
    ("compliance/model-risk-validator",  "gpt-4o",           5, (160000, 380000), 0.07),
]
ORG = "meridian.example"
recs = []
runs = []  # (run_id, agent_id, total_cost)
def add(run, ag, model, dec, cost, saved, step, ts):
    recs.append({"ts_millis": ts, "run_id": run, "model": model, "decision": dec,
                 "input_tokens": random.randint(2000, 40000), "output_tokens": random.randint(300, 3000),
                 "cost_microusd": cost, "saved_microusd": saved, "step": step, "agent_id": ag})

for team_name, model, n, (clo, chi), cache in FLEET:
    short = team_name.split("/")[-1]
    ag = f"agent://{ORG}/{team_name}"
    for i in range(n):
        run = f"{short}-{i:03d}"
        steps = random.randint(3, 12)
        total = 0
        base_ts = now - random.randint(0, 6 * 3600 * 1000)
        for s in range(steps):
            if random.random() < cache:
                add(run, ag, model, "cache_hit", 0, random.randint(clo, chi), s, base_ts + s * 1000)
            else:
                c = random.randint(clo, chi)
                total += c
                add(run, ag, model, "allow", c, random.randint(0, c // 4), s, base_ts + s * 1000)
        runs.append((run, ag, total))

# ingest
body = json.dumps({"records": recs}).encode()
req = urllib.request.Request(f"{CLOUD}/v1/ingest", data=body,
                             headers={"Authorization": f"Bearer {BEARER}", "Content-Type": "application/json"})
print("ingest:", urllib.request.urlopen(req).status, "records:", len(recs), "runs:", len(runs))

# set budgets so fuse bars read: ~half get a cap near/above spend, rest live
def post(path, obj):
    r = urllib.request.Request(f"{CLOUD}{path}", data=json.dumps(obj).encode(),
                               headers={"Authorization": f"Bearer {BEARER}", "Content-Type": "application/json"})
    try:
        return urllib.request.urlopen(r).status
    except Exception as e:
        return f"ERR {e}"

runs.sort(key=lambda x: -x[2])
for idx, (run, ag, total) in enumerate(runs):
    tot_usd = total / 1e6
    if idx % 3 == 0 and total > 0:          # over cap
        post(f"/v1/runs/{run}/budget", {"budget_usd": round(tot_usd * 0.85, 2), "reason": "mobile demo cap"})
    elif idx % 3 == 1 and total > 0:        # near cap
        post(f"/v1/runs/{run}/budget", {"budget_usd": round(tot_usd * 1.15, 2), "reason": "mobile demo cap"})
    # else: live, no cap

# kill the single biggest as a visible KILLED row
if runs:
    big = runs[0][0]
    print("kill", big, "->", post(f"/v1/runs/{big}/kill", {"reason": "mobile demo: runaway stopped"}))
print("done")
