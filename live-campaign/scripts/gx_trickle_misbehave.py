#!/usr/bin/env python3
"""A live trickle plus one instance of every way an agent can misbehave.

The seeded dataset is static, so the burn rate reads 0.00 and the behaviour
axis of the queue is empty. This keeps money flowing at a rate above the
spend_spike threshold and trips each detector the Cloud actually implements,
using its own documented thresholds (store.rs::IncidentConfig defaults):

  budget_exhausted   >= 3 budget_exceeded blocks on one run
  sustained_loop     >= 3 loop_detected decisions on one run within 10 min
  fanout_explosion   >= 20 distinct runs from one agent within 10 min
  spend_spike        org burn >= $5/min over the last minute

Nothing here fakes an incident: every one is produced by feeding the plane the
evidence its detectors look for, exactly as a gateway would.
"""
import json, random, sys, time, urllib.request

CLOUD = "http://127.0.0.1:8083"
BEARER = "devkey"
ORG = "meridian.example"
random.seed(20260721)


def post(path, payload):
    req = urllib.request.Request(
        f"{CLOUD}{path}",
        data=json.dumps(payload).encode(),
        headers={"Authorization": f"Bearer {BEARER}", "Content-Type": "application/json"},
    )
    return urllib.request.urlopen(req).status


def rec(run, agent, decision, cost, step, ts, saved=0):
    return {
        "ts_millis": ts, "run_id": run, "model": "claude-sonnet-5", "decision": decision,
        "input_tokens": random.randint(2000, 40000), "output_tokens": random.randint(300, 3000),
        "cost_microusd": cost, "saved_microusd": saved, "step": step, "agent_id": agent,
    }


def now_ms():
    return int(time.time() * 1000)


def misbehave():
    """One of each kind, on purpose, with a different agent for each so the
    per-agent join in the app has something to show."""
    out = []
    t = now_ms()

    # 1. sustained_loop: the same run deciding "loop_detected" over and over.
    loop_agent = f"agent://{ORG}/treasury/cashflow-forecaster"
    for s in range(5):
        out.append(rec("cashflow-forecaster-loop", loop_agent, "loop_detected", 0, s, t + s * 900))

    # 2. budget_exhausted: a run that keeps hitting its ceiling.
    burnt_agent = f"agent://{ORG}/kyc-aml/aml-case-copilot"
    for s in range(4):
        out.append(rec("aml-case-copilot-burnt", burnt_agent, "budget_exceeded", 0, s, t + s * 700))

    # 3. fanout_explosion: one agent spraying distinct runs.
    fan_agent = f"agent://{ORG}/support/support-tier2-bot"
    for i in range(24):
        out.append(rec(f"support-tier2-bot-fan-{i:02d}", fan_agent, "allow",
                       random.randint(20_000, 60_000), 0, t + i * 200))

    # 4. dlp_blocked and policy_violation: not detectors of their own, but real
    #    blocked outcomes that belong in the evidence a reviewer sees.
    dlp_agent = f"agent://{ORG}/compliance/model-risk-validator"
    out.append(rec("model-risk-validator-dlp", dlp_agent, "dlp_blocked", 0, 0, t))
    out.append(rec("model-risk-validator-dlp", dlp_agent, "policy_violation", 0, 1, t + 400))
    return out


def trickle_batch(seconds_of_spend):
    """Spend fast enough to keep the org's per-minute burn over the spike
    threshold ($5/min), spread across the real fleet."""
    agents = [
        ("fraud/fraud-triage-copilot", "fraud-triage-copilot"),
        ("lending/underwriting-copilot", "underwriting-copilot"),
        ("treasury/cashflow-forecaster", "cashflow-forecaster"),
    ]
    out, t = [], now_ms()
    per_call = 120_000  # $0.12
    calls = max(1, int((7_000_000 / 60) * seconds_of_spend / per_call))  # ~$7/min
    for i in range(calls):
        team, short = agents[i % len(agents)]
        out.append(rec(f"{short}-live-{i % 4:02d}", f"agent://{ORG}/{team}", "allow",
                       per_call, i, t + i * 50, saved=random.randint(0, 20_000)))
    return out


if __name__ == "__main__":
    minutes = float(sys.argv[1]) if len(sys.argv) > 1 else 3.0
    first = misbehave()
    print("misbehaviour:", post("/v1/ingest", {"records": first}), len(first), "records")
    deadline = time.time() + minutes * 60
    tick = 0
    while time.time() < deadline:
        batch = trickle_batch(10)
        post("/v1/ingest", {"records": batch})
        tick += 1
        print(f"tick {tick}: +{len(batch)} calls", flush=True)
        time.sleep(10)
    print("trickle done")
