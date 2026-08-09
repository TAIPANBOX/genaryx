#!/usr/bin/env bash
# Every number this README states about this repository, checked against the
# repository.
#
# WHY THIS EXISTS
#
# A number on a README is a claim with no owner. It is right the day it is
# written, and nothing tells anybody when it stops being right, because the
# suite grows in commits that never open the README.
#
# Not hypothetical. On 2026-08-05 the it-rat.com service pages were audited
# against the repositories they describe and FOUR OF SEVEN figures were stale:
# trailryx by 33 tests, tokenfuse by 196, engram by 42, verdryx by 75. None was
# wrong when written.
#
# Genaryx had a second version of the problem and a worse one: it stated no
# figure at all, so there was nothing to go stale and equally nothing a reader
# could hold us to.
#
# WHAT IS COUNTED, because a number needs a definition more than it needs a
# badge
#
# Every `test result:` line `cargo test --workspace` prints, summed: unit,
# integration and doc-tests, exactly what a contributor sees at the end of a
# run. The badge is therefore a figure somebody can reproduce in one command
# rather than one only this script knows how to get.
#
# It runs the suite, because cargo has no cheap enumeration equivalent to
# `go test -list`, so a red suite fails this check too. That is a side effect
# and not the point: the test step in CI is what says they pass.

set -uo pipefail

cd "$(git rev-parse --show-toplevel)" || exit 1

readme="README.md"
problems=0

note() {
	printf '%s\n' "$1"
	problems=$((problems + 1))
}

# `--no-fail-fast`, and the exit code is read rather than discarded. Without
# both, a single failing test truncates the run: cargo stops after that crate,
# the later crates never report, and the sum comes out low. This check then
# says the badge is wrong when what actually happened is that the suite broke.
#
# Measured 2026-08-09: one failing test in `connectors` cut the workspace from
# six crates to two, the sum from 688 to 479, and this printed "the badge says
# 688 tests and cargo test --all runs 479". The badge was right and the README
# was innocent.
out=$(cargo test --workspace --no-fail-fast --quiet 2>&1)
status=$?
actual=$(printf '%s\n' "$out" | grep -E '^test result' | awk '{s += $4} END {print s + 0}')

if [ "$status" -ne 0 ]; then
	note "the suite did not pass, so its count cannot be compared with anything."
	note "Fix the tests first; a failing run reports fewer tests than the repo has,"
	note "and this check would blame the README for it."
	printf '%s\n' "$out" | grep -E '^(test result|error|failures:)' | head -20
	exit 1
fi

if [ "${actual:-0}" -eq 0 ]; then
	note "the suite reported no tests at all, which means this check measured nothing"
	exit 1
fi

stated=$(grep -o 'badge/tests-[0-9]*-' "$readme" | grep -o '[0-9]*' | head -1)
if [ -z "$stated" ]; then
	note "the README carries no tests badge, so this check has nothing to compare against"
	note "add: ![tests](https://img.shields.io/badge/tests-${actual}-brightgreen)"
	exit 1
fi

[ "$stated" = "$actual" ] ||
	note "the badge says $stated tests and \`cargo test --all\` runs $actual"

if [ "$problems" -gt 0 ]; then
	printf '\n%d number(s) the README states that this repository does not support.\n' "$problems"
	printf 'Update the badge in the same commit as the tests. That is the point: the\n'
	printf 'suite changes in a commit that never opens the README, and this is what\n'
	printf 'makes that impossible.\n'
	exit 1
fi

printf '%s tests across the workspace, and the badge says so.\n' "$actual"
