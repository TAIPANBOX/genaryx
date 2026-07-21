#!/usr/bin/env python3
"""Keep the seeded fleet ALIVE, so the burn rate is a real number.

Why this exists
---------------
`gx_fleet_v3.py` seeds the fleet in one bulk ingest. After it finishes, the
org's total spend never changes again, and the relay computes

    burn_rate = (spend_now - spend_600s_ago) / elapsed * 60

so the phone and the watch both show a hero number of **0.00 $/min**. That is
arithmetically honest and completely misleading: it is the number a DEAD fleet
shows, on a pager whose entire job is to tell you something is burning.

So this trickles real calls at a believable rate, exactly as the fleet's own
agents would, until the relay's 600s window is full of genuine samples.

Two deliberate constraints
--------------------------
1. **New run ids only, and never a budgeted run.** The exception queue's
   fractions were computed from each budgeted run's settled spend
   (`gx_fleet_v3.py`), and adding spend to those runs would move the very
   numbers the screenshots are built around. Trickled traffic therefore lands
   on fresh run ids that carry no budget, so `/v1/alerts` is untouched and the
   queue stays exactly the shape it was designed to be.

2. **Stop before capturing.** Left running during a capture session, this
   reintroduces the exact defect the re-shoot exists to remove: the console read
   at 14:40 and the phone read at 14:43 would disagree. Run it, let it fill the
   window, STOP it, then capture against frozen totals. The burn rate survives
   the stop because the window is backward-looking: it keeps reporting what the
   last ten minutes actually did, which is true, and decays to zero over the
   following ten minutes.

Usage:  python3 gx_trickle.py [seconds]      (default 600, one full window)
"""
import json, random, sys, time, urllib.request

CLOUD = "http://127.0.0.1:8080"
BEARER = "devkey"
ORG = "meridian.example"
DURATION = int(sys.argv[1]) if len(sys.argv) > 1 else 600
TICK_SECS = 5

# The same agents, the same models, the same per-call costs as the seeded
# fleet, so the trickle is indistinguishable from the fleet's own traffic
# rather than a synthetic-looking add-on.
AGENTS = [
    ("fraud", "fraud-triage-copilot", "claude-sonnet-5", (70000, 140000)),
    ("fraud", "txn-anomaly-scorer", "claude-haiku-4-5", (3000, 9000)),
    ("kyc-aml", "kyc-intake-agent", "claude-sonnet-5", (60000, 130000)),
    ("kyc-aml", "sanctions-screener", "claude-haiku-4-5", (2500, 7000)),
    ("kyc-aml", "aml-case-copilot", "gpt-4o", (280000, 560000)),
    ("lending", "underwriting-copilot", "claude-sonnet-5", (80000, 170000)),
    ("support", "support-tier1-bot", "claude-haiku-4-5", (3000, 8000)),
    ("support", "support-tier2-bot", "claude-sonnet-5", (90000, 160000)),
    ("treasury", "cashflow-forecaster", "gpt-4o", (300000, 600000)),
    ("compliance", "model-risk-validator", "gpt-4o", (280000, 560000)),
]


def post(records):
    body = json.dumps({"records": records}).encode()
    req = urllib.request.Request(
        f"{CLOUD}/v1/ingest", data=body, method="POST",
        headers={"Authorization": f"Bearer {BEARER}", "Content-Type": "application/json"})
    urllib.request.urlopen(req, timeout=30).read()


def summary():
    req = urllib.request.Request(f"{CLOUD}/v1/summary",
                                 headers={"Authorization": f"Bearer {BEARER}"})
    return json.loads(urllib.request.urlopen(req, timeout=30).read())


started = time.time()
before = summary()
print(f"start:  spent=${before['spent_microusd']/1e6:,.2f}  calls={before['calls']:,}")
print(f"trickling for {DURATION}s, new unbudgeted runs only")

tick = 0
while time.time() - started < DURATION:
    tick += 1
    now_ms = int(time.time() * 1000)
    batch = []
    # A handful of calls per tick, spread across the fleet. Sized so the rate
    # lands in the same order as the seeded fleet's own average (roughly
    # $4,255 over 10 hours, about $7/min) rather than an invented figure.
    for _ in range(random.randint(6, 12)):
        team, name, model, crng = random.choice(AGENTS)
        agent = f"agent://{ORG}/{team}/{name}"
        # A SMALL, slowly-rotating pool of run ids per agent. The first version
        # of this script minted up to 41 fresh run ids per agent per minute,
        # and the fan-out detector correctly called that a `fanout_explosion`:
        # the exception queue filled with runaway signals that were artifacts
        # of the trickle, not of the fleet. Steady traffic means few runs, each
        # taking many calls, which is what ordinary agent work looks like.
        run = f"{name}-live-{now_ms // 300000}-{random.randint(0, 1)}"
        cost = random.randint(*crng)
        if random.random() < 0.18:
            batch.append({"ts_millis": now_ms, "run_id": run, "model": model,
                          "decision": "cache_hit", "input_tokens": 4000, "output_tokens": 500,
                          "cost_microusd": 0, "saved_microusd": cost, "step": 0,
                          "agent_id": agent})
        else:
            saved = int(cost * random.uniform(0.2, 0.5)) if random.random() < 0.15 else 0
            batch.append({"ts_millis": now_ms, "run_id": run, "model": model,
                          "decision": "allow", "input_tokens": random.randint(4000, 60000),
                          "output_tokens": random.randint(400, 3000),
                          "cost_microusd": cost, "saved_microusd": saved, "step": 0,
                          "agent_id": agent})
    post(batch)
    if tick % 6 == 0:
        s = summary()
        rate = (s["spent_microusd"] - before["spent_microusd"]) / 1e6 / ((time.time() - started) / 60)
        print(f"  +{int(time.time()-started):>3}s  spent=${s['spent_microusd']/1e6:,.2f}  "
              f"rate~${rate:,.2f}/min")
    time.sleep(TICK_SECS)

after = summary()
elapsed_min = (time.time() - started) / 60
added = (after["spent_microusd"] - before["spent_microusd"]) / 1e6
print(f"\nstopped. added ${added:,.2f} over {elapsed_min:.1f} min "
      f"(~${added/elapsed_min:,.2f}/min)")
print(f"final:  spent=${after['spent_microusd']/1e6:,.2f}  calls={after['calls']:,}")
print("\nCapture NOW, while the relay's 600s window still holds these samples.")
