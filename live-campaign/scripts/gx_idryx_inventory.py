#!/usr/bin/env python3
# Meridian agent + MCP inventory for idryx (--load agents:<p> --load mcp:<p>).
#
# gx_idryx.py writes the event LOG: what the agents did. This writes the
# INVENTORY: what they are allowed to call, and which MCP servers the bank
# sanctioned. Two different questions, and only the second one lets idryx see a
# tool wired to a server nobody approved - a finding the money plane cannot
# make, because nothing about it shows up in spend.
#
# Same 16 agents and the same agent:// URIs as gx_idryx.py and gx_fleet_v3.py,
# so a finding lands on an agent the phone already has a row for.
import json

ORG = "meridian.example"
def ag(team, name): return f"agent://{ORG}/{team}/{name}"

# The bank's sanctioned MCP servers, plus one that is not in the registry:
# "notes-sync" was stood up by a team for a pilot and never withdrawn.
MCP = {
    "registry": ["core-banking-mcp", "crm-mcp", "docstore-mcp"],
    "servers": [
        {"name": "core-banking-mcp", "url": "https://mcp.meridian.example/core",
         "owner": "platform-team", "tools": ["ledger_read", "txn_read"]},
        {"name": "crm-mcp", "url": "https://mcp.meridian.example/crm",
         "owner": "crm-team", "tools": ["customer_read", "case_write"]},
        {"name": "docstore-mcp", "url": "https://mcp.meridian.example/docs",
         "owner": "platform-team", "tools": ["doc_read", "doc_write"]},
        {"name": "notes-sync", "url": "https://notes-sync.vendor.example/mcp",
         "owner": "", "tools": ["notes_export", "fs_read"]},
    ],
}

# tools an agent may call, and the subset actually observed. Two agents were
# wired to the pilot server and kept the tool after it; one carries a declared
# set far wider than it ever uses.
AGENTS = [
    ("fraud", "fraud-triage-copilot", "o.marchenko", ["txn_read", "customer_read"], ["txn_read", "customer_read"]),
    ("fraud", "txn-anomaly-scorer", "o.marchenko", ["txn_read"], ["txn_read"]),
    ("kyc-aml", "kyc-intake-agent", "n.savchenko", ["doc_read", "customer_read"], ["doc_read"]),
    ("kyc-aml", "sanctions-screener", "n.savchenko", ["customer_read"], ["customer_read"]),
    ("kyc-aml", "aml-case-copilot", "d.hrytsenko", ["case_write", "customer_read", "txn_read"], ["case_write", "customer_read"]),
    ("lending", "underwriting-copilot", "s.tkachenko",
     ["doc_read", "doc_write", "customer_read", "txn_read", "ledger_read", "case_write"], ["doc_read"]),
    ("lending", "doc-intake-ocr", "s.tkachenko", ["doc_read", "doc_write", "fs_read"], ["doc_read", "fs_read"]),
    ("lending", "collateral-valuator", "i.bondar", ["doc_read"], ["doc_read"]),
    ("support", "support-tier1-bot", "a.melnyk", ["customer_read"], ["customer_read"]),
    ("support", "support-tier2-bot", "a.melnyk", ["customer_read", "case_write", "notes_export"],
     ["customer_read", "case_write", "notes_export"]),
    ("support", "escalation-router", "a.melnyk", ["case_write"], ["case_write"]),
    ("treasury", "cashflow-forecaster", "v.koval", ["ledger_read", "txn_read"], ["ledger_read", "txn_read"]),
    ("treasury", "spend-optimizer", "v.koval", ["ledger_read"], ["ledger_read"]),
    ("treasury", "reconciliation-batch", "v.koval", ["ledger_read", "txn_read", "doc_read"], ["ledger_read", "txn_read"]),
    ("compliance", "model-risk-validator", "l.romanenko", ["doc_read"], ["doc_read"]),
    ("compliance", "control-tester", "l.romanenko", ["doc_read", "case_write"], ["doc_read"]),
    ("compliance", "evidence-assembler", "l.romanenko", ["doc_read", "doc_write"], ["doc_read", "doc_write"]),
]

agents = {"agents": [
    {"id": ag(t, n), "runtime": "langgraph", "owner": owner,
     "onBehalfOf": f"user://{ORG}/{owner}", "tools": tools, "usedTools": used}
    for t, n, owner, tools, used in AGENTS
]}

for path, doc in (("/tmp/meridian-agents.json", agents), ("/tmp/meridian-mcp.json", MCP)):
    with open(path, "w") as f:
        json.dump(doc, f, indent=1)
    print(f"wrote {path}")
