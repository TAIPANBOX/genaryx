#!/usr/bin/env bash
# Enforces invariant 4 of CLAUDE.md: the console shows the operator's real
# records, never a mock.
#
# The invariant's own wording is the reason this exists: "inventing a plausible
# number to fill a panel is the single worst thing this product can do, because
# the entire proposition is that what you see is what happened."
#
# It erodes in one specific way, and it had already started. `lib/recentEvents.ts`
# fell back to the `mockData.ts` fixture stream on ANY thrown error, not only
# when there was no backend to ask. A console pointed at a real box that had
# stopped answering therefore filled its panels with fabricated agents,
# severities and timestamps. Nothing was lying about it on purpose: the catch
# block was written for the no-backend preview and quietly also caught the case
# where a real box went silent. That is how this invariant goes, every time: not
# by somebody deciding to fake data, but by one honest fallback widening.
#
# Two structural checks, and both are about WHERE fixture rows can reach:
#
#   1. Fixture ROW data (`mockData.ts`'s exports) is imported only by modules
#      on the allow-list below. A panel that imports rows directly can render
#      them next to real ones and nothing downstream can tell.
#   2. No failure path serves them. A `catch` that returns fixtures is the
#      exact shape that was there, so a `MOCK_` reference inside a catch block
#      is refused wherever it appears.
#
# What is deliberately NOT checked: `lib/mockPreview.ts` and the `MOCK` build
# flag. The mock PREVIEW build is a separate artifact (`npm run build:demo`,
# see CLAUDE.md) whose entire purpose is to have no box behind it, and it is
# labelled as demo everywhere it surfaces. The invariant is about the real
# console, not about whether a demo may exist.
#
# This file is the ONE copy of this check.

set -uo pipefail
cd "$(git rev-parse --show-toplevel)" || exit 1

WEB="apps/web/src"
problems=0

# Modules allowed to import fixture rows, and why each one is:
#   lib/recentEvents.ts  the no-backend preview, where no real answer is being
#                        displaced; it is the module this check was written
#                        around, and check 2 below is what holds its shape.
ALLOWED='^apps/web/src/lib/recentEvents\.ts$'

# ---- 1. who may import fixture rows -----------------------------------------
while IFS= read -r file; do
	[ -n "$file" ] || continue
	case "$file" in
	*.test.ts | *.test.tsx) continue ;;
	esac
	[[ "$file" =~ $ALLOWED ]] && continue
	echo "FAIL: $file imports fixture rows from mockData"
	echo "      A panel holding fixture rows can render them beside real ones,"
	echo "      and nothing downstream can tell which is which."
	problems=$((problems + 1))
done < <(grep -rlE "from ['\"].*mockData['\"]" --include='*.ts' --include='*.tsx' "$WEB" 2>/dev/null)

# ---- 2. no failure path serves them -----------------------------------------
# A `catch` that reaches for fixtures is the defect this invariant lost to.
# Scanned per file with awk so the brace depth of the catch block is tracked
# rather than guessed at with a line-distance window.
while IFS= read -r hit; do
	[ -n "$hit" ] || continue
	echo "FAIL: $hit"
	echo "      A read that failed has no records to show. Saying so is the"
	echo "      product; filling the panel with plausible ones is not."
	problems=$((problems + 1))
done < <(
	find "$WEB" -name '*.ts' -o -name '*.tsx' 2>/dev/null |
		grep -v '\.test\.' |
		grep -v 'mockPreview\.ts$' |
		while IFS= read -r f; do
			awk -v F="$f" '
			/catch[[:space:]]*\(/ { incatch = 1; depth = 0 }
			incatch {
				n = gsub(/\{/, "{"); depth += n
				m = gsub(/\}/, "}"); depth -= m
				if (/MOCK_/) print F ":" NR ": a catch block reaches for fixture data:" $0
				if (depth <= 0 && seen_open) { incatch = 0; seen_open = 0 }
				if (depth > 0) seen_open = 1
			}
		' "$f"
		done
)

if [ "$problems" -ne 0 ]; then
	echo
	echo "A panel that invents a plausible number has broken the only promise"
	echo "this product makes. See CLAUDE.md invariant 4."
	exit 1
fi

echo "OK: fixture rows reach no panel and no failure path."
