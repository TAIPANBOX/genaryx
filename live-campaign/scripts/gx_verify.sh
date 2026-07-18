#!/usr/bin/env bash
A=(-H "Authorization: Bearer devkey"); B=http://127.0.0.1:8080
echo "=== SUMMARY ==="; curl -s "${A[@]}" $B/v1/summary | jq .
echo "=== SAVINGS ==="; curl -s "${A[@]}" $B/v1/savings | jq .
echo "=== one raw agent (field check) ==="; curl -s "${A[@]}" $B/v1/agents | jq '.[0]'
echo "=== TOP AGENTS by spend ==="
curl -s "${A[@]}" $B/v1/agents | jq -r 'sort_by(-.spent_microusd)|.[:10][]|"  \(.spent_microusd/1000000|floor) USD  \(.calls) calls  \(.agent_id)"'
echo "=== INCIDENTS (by kind) ==="
curl -s "${A[@]}" $B/v1/incidents | jq -r '.[]|.kind' | sort | uniq -c | sort -rn
echo "=== INCIDENTS (top 8 raw) ==="
curl -s "${A[@]}" $B/v1/incidents | jq -r '.[:8][]|"  \(.kind) [\(.severity)] run=\(.run_id) x\(.occurrences)"'
echo "=== ALERTS count ==="; curl -s "${A[@]}" $B/v1/alerts | jq 'length'
