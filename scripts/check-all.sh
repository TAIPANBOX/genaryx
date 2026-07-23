#!/usr/bin/env bash
# The whole-repo gate: everything that must still build and pass before a
# change leaves this machine.
#
#   bash scripts/check-all.sh            full gate (what CI runs)
#   bash scripts/check-all.sh --fast     compile-level only, for the pre-push hook
#   bash scripts/check-all.sh --list     print the steps and exit
#
# WHY THIS EXISTS, and why `cargo test --workspace` is not enough.
#
# The console's frontend is TypeScript, and `cargo build/clippy/test
# --workspace` never compiles a line of it. This script exists so the parts
# cargo cannot see are still checked explicitly, on every run, locally, with
# no network and no billing in the path. (Its original motivation was the
# standalone Tauri shell's Cargo project, which the workspace could not see
# either; the desktop shells are gone since the web-only pivot, but the
# principle stayed and the frontend still needs it.)
#
# Everything here is read-only with respect to source: it compiles, lints and
# tests, and never rewrites a file (`cargo fmt` runs in `--check` mode).

set -uo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

FAST=0
LIST=0
for arg in "$@"; do
  case "$arg" in
    --fast) FAST=1 ;;
    --list) LIST=1 ;;
    -h|--help) sed -n '2,8p' "${BASH_SOURCE[0]}"; exit 0 ;;
    *) echo "check-all: unknown argument '$arg'" >&2; exit 2 ;;
  esac
done

# ---------------------------------------------------------------------------
# Step bookkeeping. Deliberately NOT fail-fast: when several things are broken
# at once (the normal case after a core change), one run should report all of
# them rather than making the operator play whack-a-mole one push at a time.
# ---------------------------------------------------------------------------
PASSED=(); FAILED=(); SKIPPED=()
started_at=$SECONDS

step() {
  local name="$1"; shift
  local dir="$1"; shift
  if [ "$LIST" -eq 1 ]; then echo "  $name  (in ${dir#$ROOT/})"; return 0; fi

  local t0=$SECONDS
  printf '\n\033[1m==> %s\033[0m  (%s)\n' "$name" "${dir#$ROOT/}"
  if ( cd "$dir" && "$@" ); then
    PASSED+=("$name")
    printf '\033[32m    ok\033[0m  %ss\n' "$((SECONDS - t0))"
  else
    FAILED+=("$name")
    printf '\033[31m    FAILED\033[0m  %ss\n' "$((SECONDS - t0))"
  fi
}

skip() {
  local name="$1" why="$2"
  if [ "$LIST" -eq 1 ]; then echo "  $name  (skipped: $why)"; return 0; fi
  SKIPPED+=("$name ($why)")
  printf '\n\033[1m==> %s\033[0m\n\033[33m    skipped: %s\033[0m\n' "$name" "$why"
}

[ "$LIST" -eq 1 ] && echo "check-all steps ($([ $FAST -eq 1 ] && echo fast || echo full)):"

# ---------------------------------------------------------------------------
# 1. The workspace: crates/* only. This is the part `--workspace` covers.
# ---------------------------------------------------------------------------
step "workspace: fmt"    "$ROOT" cargo fmt --all --check
step "workspace: clippy" "$ROOT" cargo clippy --workspace --all-targets -- -D warnings
if [ "$FAST" -eq 1 ]; then
  skip "workspace: test" "--fast"
else
  step "workspace: test" "$ROOT" cargo test --workspace
fi

# ---------------------------------------------------------------------------
# 2. The web console's frontend. The blind spot this script exists for:
#    TypeScript that no cargo invocation will ever compile.
# ---------------------------------------------------------------------------
WEB="$ROOT/apps/web"
if ! command -v pnpm >/dev/null 2>&1; then
  skip "web ui" "pnpm not installed"
else
  # `--frozen-lockfile` is the CI behaviour and the one we want locally too:
  # it fails rather than silently resolving a different tree than the one
  # committed. Skipped in --fast when node_modules is already present, since
  # a pre-push hook that reinstalls dependencies is a hook nobody keeps.
  if [ "$FAST" -eq 1 ] && [ -d "$WEB/node_modules" ]; then
    skip "web ui: pnpm install" "--fast, node_modules present"
  else
    step "web ui: pnpm install" "$WEB" pnpm install --frozen-lockfile
  fi
  step "web ui: tsc" "$WEB" pnpm exec tsc --noEmit
  if [ "$FAST" -eq 1 ]; then
    skip "web ui: vitest" "--fast"
    skip "web ui: vite build" "--fast"
  else
    step "web ui: vitest" "$WEB" pnpm test
    step "web ui: vite build" "$WEB" pnpm build
  fi
fi

[ "$LIST" -eq 1 ] && exit 0

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------
printf '\n\033[1m---- check-all (%s) ----\033[0m\n' "$([ $FAST -eq 1 ] && echo fast || echo full)"
printf 'passed:  %d\n' "${#PASSED[@]}"
if [ "${#SKIPPED[@]}" -gt 0 ]; then
  printf 'skipped: %d\n' "${#SKIPPED[@]}"
  for s in "${SKIPPED[@]}"; do printf '  - %s\n' "$s"; done
fi
printf 'elapsed: %ss\n' "$((SECONDS - started_at))"

if [ "${#FAILED[@]}" -gt 0 ]; then
  printf '\033[31mfailed:  %d\033[0m\n' "${#FAILED[@]}"
  for f in "${FAILED[@]}"; do printf '\033[31m  - %s\033[0m\n' "$f"; done
  exit 1
fi

printf '\033[32mall green\033[0m\n'
