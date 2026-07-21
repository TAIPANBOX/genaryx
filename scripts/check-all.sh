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
# `apps/desktop/src-tauri` is deliberately a STANDALONE Cargo project, not a
# member of the root workspace (see its Cargo.toml: the empty `[workspace]`
# table stops upward discovery so it locks and builds independently). That is
# the right call, but it has a sharp edge: `cargo build/clippy/test
# --workspace` at the root NEVER compiles the Tauri shell. On 2026-07-21 that
# edge drew blood. Commit 4ad8b83 added an `InvalidPathSegment` variant to
# `ConnectorError` and `WardryxError` in `crates/connectors`, updated
# `crates/ffi` (which the SwiftUI shell consumes, and which IS in the
# workspace), and left the Tauri shell with four non-exhaustive `match`
# expressions. The workspace stayed green. The shell did not compile at all,
# and nothing said so, because the one CI job that would have caught it had
# not run since 2026-07-17 (GitHub Actions blocked on billing for private
# repos).
#
# So the rule this script encodes: a shell that the workspace cannot see must
# be checked explicitly, on every run, locally, with no network and no
# billing in the path. Keeping src-tauri out of the workspace stays correct;
# THIS SCRIPT, not workspace membership, is the guarantee.
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
# 2. The Tauri shell's Rust. The blind spot this script exists for.
#    In --fast we still COMPILE it (cargo check), because "does the shell
#    build against the core as it stands right now" is exactly the question
#    that went unanswered for four days.
# ---------------------------------------------------------------------------
TAURI_RS="$ROOT/apps/desktop/src-tauri"
if [ "$FAST" -eq 1 ]; then
  step "tauri shell: cargo check" "$TAURI_RS" cargo check --all-targets
else
  step "tauri shell: clippy" "$TAURI_RS" cargo clippy --all-targets -- -D warnings
  step "tauri shell: test"   "$TAURI_RS" cargo test
fi

# ---------------------------------------------------------------------------
# 3. The Tauri shell's frontend.
# ---------------------------------------------------------------------------
DESKTOP="$ROOT/apps/desktop"
if ! command -v pnpm >/dev/null 2>&1; then
  skip "tauri shell: frontend" "pnpm not installed"
else
  # `--frozen-lockfile` is the CI behaviour and the one we want locally too:
  # it fails rather than silently resolving a different tree than the one
  # committed. Skipped in --fast when node_modules is already present, since
  # a pre-push hook that reinstalls dependencies is a hook nobody keeps.
  if [ "$FAST" -eq 1 ] && [ -d "$DESKTOP/node_modules" ]; then
    skip "tauri shell: pnpm install" "--fast, node_modules present"
  else
    step "tauri shell: pnpm install" "$DESKTOP" pnpm install --frozen-lockfile
  fi
  step "tauri shell: tsc" "$DESKTOP" pnpm exec tsc --noEmit
  if [ "$FAST" -eq 1 ]; then
    skip "tauri shell: vite build" "--fast"
  else
    step "tauri shell: vite build" "$DESKTOP" pnpm build
  fi
fi

# ---------------------------------------------------------------------------
# 4. The SwiftUI shell. macOS only, and expensive: build-ffi.sh regenerates
#    the UniFFI bindings and the xcframework from scratch every time (it is
#    idempotent by wiping both output directories), so it is a full-gate step
#    rather than a pre-push one.
#
#    Note this covers a DIFFERENT failure than step 2. `crates/ffi` is in the
#    workspace, so a Rust-side break there is caught by step 1; what only
#    this step catches is Swift code that no longer matches a changed FFI
#    interface.
# ---------------------------------------------------------------------------
MACOS="$ROOT/apps/macos"
if [ "$(uname -s)" != "Darwin" ]; then
  skip "swiftui shell" "not macOS"
elif [ "$FAST" -eq 1 ]; then
  skip "swiftui shell" "--fast (xcframework regeneration is minutes)"
elif ! command -v swift >/dev/null 2>&1; then
  skip "swiftui shell" "swift not installed"
else
  step "swiftui shell: ffi bindings" "$MACOS" bash build-ffi.sh
  step "swiftui shell: swift build"  "$MACOS" swift build
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
