#!/usr/bin/env python3
# Seed wardryx (policy plane) with meridian policy-as-code + pending approvals.
# Pure HTTP against the running wardryx (no restart). Admin bearer = devkey.
import json, urllib.request, urllib.error
W = "http://127.0.0.1:8090"
H = {"Authorization": "Bearer devkey", "Content-Type": "application/json"}
ORG = "meridian.example"

def req(method, path, body=None):
    data = json.dumps(body).encode() if body is not None else None
    r = urllib.request.Request(W + path, data=data, method=method, headers=H)
    try:
        resp = urllib.request.urlopen(r, timeout=15)
        return resp.status, resp.read().decode()[:300]
    except urllib.error.HTTPError as e:
        return e.code, e.read().decode()[:300]

policies = [
    ("treasury-human-approval", {"name": "treasury-human-approval", "target": f"agent://{ORG}/treasury/*", "require_human_above_usd": 25.0}),
    ("underwriting-approval", {"name": "underwriting-approval", "target": f"agent://{ORG}/lending/underwriting-copilot", "require_human_above_usd": 10.0}),
    ("deny-shell-exec", {"name": "deny-shell-exec", "target": f"agent://{ORG}/*", "deny_tool": ["shell_exec", "file_write"]}),
    ("kyc-require-attestation", {"name": "kyc-require-attestation", "target": f"agent://{ORG}/kyc-aml/*", "deny_if_unattested": True}),
    ("support-spend-cap", {"name": "support-spend-cap", "target": f"agent://{ORG}/support/*", "deny_above_usd": 5.0}),
    ("aml-max-steps", {"name": "aml-max-steps", "target": f"agent://{ORG}/kyc-aml/aml-case-copilot", "max_steps": 12}),
]
for pid, body in policies:
    print("PUT", pid, req("PUT", f"/v1/policies/{pid}", body))

# decides that trip require_human_above_usd -> create pending approvals (the inbox)
decides = [
    # The protagonist run, so the approval on the Policy tab names the same id
    # the console, the phone, the watch and Felyx all point at. It referenced a
    # previous generation's shard (eod-001-s042) until 2026-07-20.
    ("reconciliation-batch", "treasury", "reconciliation-batch-eod-002-LIVE", "v.koval", "gpt-4o", 48.0, ["ledger_read", "gl_post"]),
    ("cashflow-forecaster", "treasury", "cashflow-forecaster-0180", "v.koval", "gpt-4o", 31.5, ["market_data"]),
    ("underwriting-copilot", "lending", "underwriting-copilot-0231", "s.tkachenko", "claude-sonnet-5", 12.4, ["credit_bureau"]),
    ("spend-optimizer", "treasury", "spend-optimizer-0044", "v.koval", "claude-sonnet-5", 27.0, ["vendor_api"]),
    ("underwriting-copilot", "lending", "underwriting-copilot-0238", "s.tkachenko", "claude-sonnet-5", 15.8, ["credit_bureau", "kyc_lookup"]),
]
for name, team, run, user, model, cost, tools in decides:
    body = {"agent_id": f"agent://{ORG}/{team}/{name}", "run_id": run,
            "on_behalf_of": [f"user://{ORG}/{user}"], "model": model,
            "est_cost_usd": cost, "tool_names": tools, "steps": 5}
    print("DECIDE", name, req("POST", "/v1/decide", body))

print("POLICIES:", req("GET", "/v1/policies"))
print("APPROVALS:", req("GET", "/v1/approvals"))
