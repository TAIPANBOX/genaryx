#!/usr/bin/env bash
# Mockryx fire drill against the LIVE gateway (the drills plane).
#
# What this is for
# ----------------
# NEXT-CAMPAIGN.md section 6 lists drills as one of the four planes that were
# empty in the 2026-07-19 capture set. The honest way to fill it is not to write
# rows into a store, it is to actually rehearse the guardrail and record what
# happened, so `mockryx run` is invoked for real against the running stack.
#
# The scenario that matters is the one that replays THIS campaign's own
# incident: `runaway-budget.yaml` gives an agent a tiny budget and tells it to
# keep looping, and expects the budget Breaker to cut it off with HTTP 402 well
# before it can run away. That is the same shape as
# `reconciliation-batch-eod-002-LIVE` retrying an oversized ledger context until
# its per-run ceiling tripped. Passing means the guardrail held.
#
# IMPORTANT: this writes to the campaign dataset
# ----------------------------------------------
# Mockryx sends REAL requests to the gateway on /v1/messages, which the gateway
# meters and ships to the Cloud like any other traffic. So this run adds real
# calls, real decisions and a real agent (`agent://mockryx.local/rehearsal/*`)
# to the same fleet the screenshots show. That is correct and honest, the drill
# genuinely happened, but it means:
#
#   * run this BEFORE the final `gx_verify.sh` dump, never after, or the numbers
#     in the article will not match what the console shows (capture protocol,
#     NEXT-CAMPAIGN.md section 7);
#   * every re-run adds more, so do not run it repeatedly to "check something".
#
# Where it runs
# -------------
# From the Mac, through the SSH tunnel to the box (gateway on 127.0.0.1:4100),
# because Genaryx invokes the same binary from the same Mac and reads the
# gateway URL out of `~/.taipan/environments/genaryx-live.json`. Running it here
# means the Drills tab shows a report produced exactly the way the console
# would produce it.
set -uo pipefail

MOCKRYX="${MOCKRYX:-$HOME/.taipan/bin/mockryx}"
SCENARIOS="${SCENARIOS:-$HOME/Development/mockryx/scenarios}"
GATEWAY="${GATEWAY:-http://127.0.0.1:4100}"
OUT_DIR="${OUT_DIR:-$HOME/Development/genaryx/live-campaign/drills}"
ONLY_RUNAWAY="${ONLY_RUNAWAY:-0}"   # 1 = rehearse just the runaway pattern

[ -x "$MOCKRYX" ] || { echo "no mockryx binary at $MOCKRYX"; exit 1; }
[ -d "$SCENARIOS" ] || { echo "no scenarios at $SCENARIOS"; exit 1; }

# Fail early and loudly if the tunnel is down, rather than producing a report
# full of transport errors that looks like a guardrail failure.
if ! curl -s -o /dev/null -m 5 "$GATEWAY/" ; then
  echo "gateway unreachable at $GATEWAY (is the SSH tunnel up?)"
  exit 1
fi

mkdir -p "$OUT_DIR"
STAMP="$(date -u +%Y%m%dT%H%M%SZ)"
REPORT="$OUT_DIR/drill-$STAMP.json"

# Mockryx loads a DIRECTORY non-recursively, so rehearsing one scenario in
# isolation means handing it a directory containing only that file.
TARGET="$SCENARIOS"
if [ "$ONLY_RUNAWAY" = "1" ]; then
  TARGET="$(mktemp -d)"
  cp "$SCENARIOS/runaway-budget.yaml" "$TARGET/"
fi

echo "rehearsing against $GATEWAY"
echo "  scenarios: $TARGET"
"$MOCKRYX" run --gateway "$GATEWAY" --format json --save "$REPORT" "$TARGET"
EXIT=$?

# Exit 0 = every guardrail held. Exit 1 = a real gap was found, which is a
# legitimate result to report, not a script failure. Exit 2 = usage/config.
case "$EXIT" in
  0) VERDICT="every rehearsed guardrail HELD" ;;
  1) VERDICT="GAPS FOUND, read the findings below" ;;
  *) VERDICT="mockryx could not run (exit $EXIT)" ;;
esac
echo
echo "verdict: $VERDICT"
echo "report:  $REPORT"

if [ -s "$REPORT" ]; then
  python3 - "$REPORT" <<'PY'
import json, sys
r = json.load(open(sys.argv[1]))
print(f"\nrun_id: {r.get('run_id')}  gateway: {r.get('gateway')}")
for res in r.get("results", []):
    m = res.get("metrics") or {}
    print(f"  {res['status']:<24} {res['scenario']:<28} "
          f"calls={m.get('calls', 0):<4} burned=${m.get('budget_burned_usd', 0):.4f}")
    for f in res.get("findings", []):
        print(f"      gap: step={f.get('step')} expected={f.get('expect_status')} "
              f"got={f.get('got_status')} {f.get('detail','')}")
PY
fi
exit "$EXIT"
