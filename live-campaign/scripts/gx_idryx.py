#!/usr/bin/env python3
# Meridian identity-graph event log for idryx (--load tokenfuse:<path>).
# NDJSON agent-event envelopes; idryx builds identities from agent_id + on_behalf_of,
# labels Source from each line's `source`, counts events/alerts, and fires detectors
# (runaway_agent off budget_exhausted, policy denials, etc.). Consistent with the
# money fleet: org meridian.example, same 16 agents + the reconciliation runaway.
import json, random, datetime as dt
random.seed(20260717)
ORG = "meridian.example"
base = dt.datetime(2026, 7, 17, 8, 0, 0, tzinfo=dt.timezone.utc)
def ts(off_s): return (base + dt.timedelta(seconds=off_s)).strftime("%Y-%m-%dT%H:%M:%SZ")

# team, name, human owner (non-human identity acts on behalf of a human operator)
FLEET = [
 ("fraud",      "fraud-triage-copilot", "o.marchenko"),
 ("fraud",      "txn-anomaly-scorer",   "o.marchenko"),
 ("kyc-aml",    "kyc-intake-agent",     "n.savchenko"),
 ("kyc-aml",    "sanctions-screener",   "n.savchenko"),
 ("kyc-aml",    "aml-case-copilot",     "d.hrytsenko"),
 ("lending",    "underwriting-copilot", "s.tkachenko"),
 ("lending",    "doc-intake-ocr",       "s.tkachenko"),
 ("lending",    "collateral-valuator",  "i.bondar"),
 ("support",    "support-tier1-bot",    "a.melnyk"),
 ("support",    "support-tier2-bot",    "a.melnyk"),
 ("support",    "escalation-router",    "a.melnyk"),
 ("treasury",   "cashflow-forecaster",  "v.koval"),
 ("treasury",   "spend-optimizer",      "v.koval"),
 ("compliance", "model-risk-validator", "l.romanenko"),
 ("compliance", "control-tester",       "l.romanenko"),
 ("compliance", "evidence-assembler",   "l.romanenko"),
]
lines = []
def ev(source, typ, sev, agent, obo, off):
    lines.append(json.dumps({"schema": "taipanbox.dev/agent-event/v0.2", "ts": ts(off),
        "source": source, "type": typ, "severity": sev, "agent_id": agent, "on_behalf_of": obo}))

# steady-state activity: each agent acts on behalf of its human owner
for team, name, owner in FLEET:
    ag = f"agent://{ORG}/{team}/{name}"; human = [f"user://{ORG}/{owner}"]
    for i in range(random.randint(25, 60)):
        ev("tokenfuse", "call", "info", ag, human, i * 47 + random.randint(0, 40))
# a delegation edge: tier2 acts on behalf of tier1 bot (agent->agent chain)
ev("tokenfuse", "call", "info", f"agent://{ORG}/support/support-tier2-bot",
   [f"agent://{ORG}/support/support-tier1-bot", f"user://{ORG}/a.melnyk"], 900)

# governance / detector signal --------------------------------------------
# runaway: reconciliation batch fanned out, budget ceiling tripped repeatedly
rec = f"agent://{ORG}/treasury/reconciliation-batch"
for i in range(40):
    ev("tokenfuse", "budget_exhausted", "critical", rec, [f"user://{ORG}/v.koval"], 3000 + i * 12)
# policy denials: underwriting tried a disallowed data source
uw = f"agent://{ORG}/lending/underwriting-copilot"
for i in range(6):
    ev("wardryx", "policy_deny", "high", uw, [f"user://{ORG}/s.tkachenko"], 1500 + i * 90)
# DLP: tier1 support bot blocked exfiltrating PII
t1 = f"agent://{ORG}/support/support-tier1-bot"
for i in range(9):
    ev("wardryx", "dlp_block", "high", t1, [f"user://{ORG}/a.melnyk"], 2000 + i * 55)
# fire-drill rehearsal identities (mockryx) - separate namespace, governed by demo policy
for i in range(4):
    ev("mockryx", "drill", "info", f"agent://mockryx.local/rehearsal/worker-{i}", [], 2600 + i * 30)

random.shuffle(lines)
out = "/tmp/meridian-idryx.ndjson"
with open(out, "w") as f:
    f.write("\n".join(lines) + "\n")
ids = set()
for l in lines:
    o = json.loads(l); ids.add(o["agent_id"])
    for x in o["on_behalf_of"]:
        ids.add(x)
print(f"wrote {len(lines)} events, {len(ids)} identities -> {out}")
