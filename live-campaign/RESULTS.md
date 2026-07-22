# Genaryx live-validation campaign - results archive

> **SUPERSEDED for citations (2026-07-20).** This file is the archive of the first
> full campaign (2026-07-17/18) and of its epoch: $4,314.42 spend, 9,287 runs,
> 176 incidents, $2,992.70 saved, 29 identities / 44 alerts. A live re-run on
> 2026-07-20 re-froze everything to one epoch, **e01**, and that is what the site,
> the articles, the briefs and the explainer cite now:
> **$4,671.02 spend / 9,501 runs / 37,596 calls / 15 killed / 180 incidents
> (181 budget breaks) / $3,131.82 saved / 29 identities, 43 alerts.**
> The e01 record lives in `VERIFICATION-LOG.md` and `shots/2026-07-20/MANIFEST.md`;
> the re-run also fixed the phone/console dataset split (the relay now serves a
> bounded slice of the same fleet the console reads). Keep this file as the
> runbook and the history; do not cite its numbers.

Full record of the live Hetzner run of the whole TAIPANBOX stack under Genaryx.
Everything here is reusable: the exact dataset generators, the verified numbers,
the deployment method, and a reproduction runbook. Captured so the campaign can
be re-run on a fresh box and the numbers cited in the enterprise article.

## Status

- **Ran:** 2026-07-17 (evening) into 2026-07-18.
- **Box:** Hetzner **CPX62** (`ubuntu-32gb-fsn1-1`), IP `5.75.234.176`.
- **Torn down:** 2026-07-18 by Yurii (standing rule: he provisions + deletes the box + key).
  The box is GONE; the app can no longer pair. Re-run from the runbook below on a fresh box.
- **SSH key:** `~/.ssh/hetzner-genaryx-20260717` (ed25519, still on disk, NOT deleted).
  Pubkey: `ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIHZxB6pqqbZ77F7HVZe6fAG3HT3rhRJ+S+fj78YHso4r genaryx-live-validation-20260717`

## The scenario

Fictional tier-1 bank, org **`meridian.example`**: 16 production AI agents across
fraud / kyc-aml / lending / support / treasury / compliance, plus one runaway
end-of-day reconciliation batch. The whole open stack (tokenfuse gateway+cloud,
wardryx, idryx, qryx, mockryx, engram, verdryx) deployed on the box via `stack-up`;
Genaryx run on the Mac against it over an SSH tunnel.

## Verified live numbers (money plane - tokenfuse cloud)

`GET /v1/summary` + `/v1/savings` (bearer `devkey`, org `default`):

| Metric | Raw (microUSD) | Value |
|---|---|---|
| Actual AI spend | `4,314,419,916` | **$4,314.42** |
| Prevented (budget kill-switch) | `2,370,403,389` | **$2,370.40** |
| Semantic-cache savings | `358,199,507` | **$358.20** |
| Model-router savings | `264,100,789` | **$264.10** |
| Total governed savings | `2,992,703,685` | **$2,992.70** |
| Budget breaks | - | **180** |
| Runs | - | **9,287** |
| Calls | - | **34,824** |
| Incidents | - | **176** (174 `budget_exhausted` + 2 `fanout_explosion`) |

Enterprise framing: ~$4.3k/day spend (~$130k/mo); on the day the reconciliation
batch went rogue, governance prevented ~$2,340 that would otherwise have hit the
invoice, and recovered ~41% of gross draw.

### Top agents by spend (`GET /v1/agents`)

| Agent | Team | Spend | Calls |
|---|---|---|---|
| aml-case-copilot | kyc-aml | $1,283.74 | 3,256 |
| cashflow-forecaster | treasury | $1,038.95 | 2,401 |
| model-risk-validator | compliance | $583.05 | 1,475 |
| underwriting-copilot | lending | $441.61 | 3,861 |
| fraud-triage-copilot | fraud | $279.88 | 3,039 |
| support-tier2-bot | support | $207.49 | 2,144 |
| kyc-intake-agent | kyc-aml | $189.86 | 2,254 |
| **reconciliation-batch** | treasury | **$85.57** | **4,350** |
| evidence-assembler | compliance | ~$80 | 761 |
| spend-optimizer | treasury | ~$67 | 719 |

The runaway `reconciliation-batch` is the tell: only **$85 actually spent** but
**4,350 calls** - near-invisible by money because governance stopped it before it
could burn, glaring by volume. Modeled as `reconciliation-batch-eod-001` fanned
into 150 shards, each retrying an oversized ledger context; the per-run budget
ceiling tripped repeatedly (`budget_exhausted`) and the fan-out raised
`fanout_explosion`.

## Policy plane (wardryx :8090)

6 meridian policies + 5 pending approvals (all seeded live over HTTP, bearer `devkey`):

- Policies: `treasury-human-approval` (require human > $25), `underwriting-approval`
  (> $10), `deny-shell-exec` (deny `shell_exec`,`file_write`),
  `kyc-require-attestation` (deny if unattested), `support-spend-cap` (deny > $5),
  `aml-max-steps` (max 12 steps).
- 5 pending approvals from `/v1/decide` calls that tripped `require_human_above_usd`,
  e.g. "estimated cost $48.00 exceeds policy treasury-human-approval threshold
  $25.00; human approval required" (reconciliation-batch, cashflow-forecaster,
  underwriting x2, spend-optimizer).

## Identity plane (idryx :8082, tunneled to the Mac as :8081)

29 identities (16 agents + service accounts + human owners), 44 detector alerts,
785 identity events ingested. Top by events: support-tier2-bot (59 ev, 2 al),
kyc-intake-agent (58, 2), txn-anomaly-scorer (56, 2), support-tier1-bot (53, 3),
model-risk-validator (50, 2). Fed from a crafted agent-event NDJSON via
`idryx serve --load tokenfuse:<file>`.

## Local-exec planes (on the Mac, on-demand)

qryx (crypto/PQC) and mockryx (drills) Mac binaries built into `~/.taipan/bin`;
engram-mcp symlinked there. Verdryx (quality) + engram (memory) stores were NOT
seeded this run (empty). qryx NCSC PQC scan verified working locally.

## Grounding facts (so the numbers are defensible, not invented)

From tokenfuse cloud `crates/cloud/src/store.rs`:
- `spent_microusd` = sum of `cost_microusd` over NON-blocked rows only (`allow`;
  `cache_hit` cost is 0). `is_blocked(d) = !matches!(d, "allow"|"cache_hit")`.
- `/v1/savings.blocked_spend_microusd` = sum of `cost_microusd` over
  BUDGET-PROTECTION reasons only: `budget_exceeded, loop_detected,
  policy_violation, wasm_policy, killed` (dlp/taint excluded - security, cost 0).
- `cache_saved` = `saved_microusd` on `cache_hit` rows; `router_saved` =
  `saved_microusd` on `allow` rows. Wire DTO is `CallRecord`.
- Caps: `MAX_RUNS_PER_ORG = 50,000`, `MAX_INCIDENTS_PER_ORG = 10,000`.
- Cloud accepts `devkey` because it was started with `TOKENFUSE_CLOUD_ALLOW_DEVKEY=1`
  (org resolves to `default`; `meridian.example` lives only inside the agent_id strings).

## Scripts (`scripts/`)

- `gx_fleet_v2.py` - the enterprise money-plane generator. 16 agents + the 150-shard
  runaway; realistic per-call costs; self-projects the totals before POSTing to
  cloud `/v1/ingest`. **This is the authoritative dataset.**
- `gx_idryx.py` - meridian identity-graph agent-event NDJSON generator (for idryx `--load`).
- `gx_policy_seed.py` - wardryx policies + `/v1/decide` approvals seeder (pure HTTP).
- `gx_deploy.sh`, `gx_setup.sh` - box toolchains + `stack-up` deployment.
- `gx_verify.sh` - live metric verification (summary/savings/agents/incidents).
- `gx_relaunch.sh`, `gx_idryx_launch.sh` - service (re)launch helpers (PATH-fixed,
  detached-wrapper pattern to avoid the ssh-channel-hang gotcha).
- `genaryx-live.descriptor.json` - the `~/.taipan/environments/` descriptor that
  points Genaryx at the (tunneled) box services.

## Screenshots (`shots/`)

- `native-overview-redesign.png` - the redesigned NATIVE SwiftUI Overview dashboard
  (hero $4,314 + burn sparkline + governance fuse, KPI tiles, spend-by-agent bars,
  savings composition). **The hero shot.**
- `native-money-redesign.png` - the redesigned native Money dashboard (interactive
  runs board with Replay/Budget/Kill, incidents rail with Ack, savings composition).
- `native-baseline.png` - pre-redesign baseline (old chip + 4 tiles), same live data.
- `money-01-runs.png` - the earlier Tauri-shell Money runs table.

## Reproduction runbook (fresh box)

1. Yurii provisions a fresh Hetzner CPX62-class box with a fresh public SSH key,
   hands me the IP; he tears it down after.
2. On the box: run `gx_setup.sh` (Rust/Go/Python toolchains) then `gx_deploy.sh`
   (clones the public TAIPANBOX repos + `stack-up` builds + starts the stack:
   gateway :4100, cloud :8080, wardryx :8090, idryx :8081, dashboard :3000).
3. Seed:
   - Money: `python3 gx_fleet_v2.py` (injects to cloud `/v1/ingest`).
   - Identity: `python3 gx_idryx.py` -> NDJSON, then a SEPARATE
     `idryx serve --addr 127.0.0.1:8082 --load tokenfuse:<ndjson>` (do NOT kill the
     stack-up idryx - stack-up tears the whole stack down if a child dies, flushing
     the in-memory cloud).
   - Policy: `python3 gx_policy_seed.py`.
4. SSH tunnel from the Mac:
   `ssh -N -L 8080:127.0.0.1:8080 -L 8090:127.0.0.1:8090 -L 8081:127.0.0.1:8082 -L 4100:127.0.0.1:4100 root@<box>`
   (note 8081 -> box 8082 = the meridian idryx).
5. `~/.taipan/environments/genaryx-live.json` (see `scripts/genaryx-live.descriptor.json`)
   + `genaryx-live.keys.json` `{ "secrets": { "cloud_admin": "devkey", "wardryx_admin": "devkey" } }` (chmod 600).
6. Build + run Genaryx (native shell): `cd apps/macos && bash build-ffi.sh && swift build`,
   then run. It reads the descriptor and shows the live data.

## Gotchas learned

- The cloud is IN-MEMORY: restarting the stack flushes it; re-inject after any restart.
- `stack-up` monitors all children and runs full cleanup if any dies -> never kill a
  single stack service; run extra instances (e.g. meridian idryx) on a new port.
- `setsid nohup ./cmd &` directly in an interactive ssh command hangs the channel
  (exit 255); launch a detached WRAPPER script instead.
- A `swift run` / bare binary launched from a shell may not register a window with the
  window server; wrap the binary in a minimal `.app` bundle (`apps/macos/Genaryx.app`)
  to get a proper window + let computer-use target it. Capture reliably with
  `screencapture -l<windowID>` (Quartz window id), which ignores z-order.
