#!/usr/bin/env bash
# Every command the web console sends is one genaryx-web handles, or one it
# says on purpose that only the mock preview answers (CLAUDE.md invariant 50).
#
# WHY THIS EXISTS
#
# The console reaches the box through `invokeBackend("<name>", ...)` and a
# dozen per-plane wrappers (`call`, `callWithCeremony`, `act`, `edit`). A name
# the box has no arm for falls to dispatch's fallback and answers 404 "unknown
# command"; nearly every caller swallows that into `null`. So a button built
# on such a name did nothing and said nothing: the agent card's owner and unit
# pickers read `org_directory`, which no box ever answered. Nothing compared
# the names the frontend sends with the names the backend handles.
#
# WHAT IT HOLDS
#
#  1. Every command name the frontend passes as a quoted first argument to a
#     call, in a module that reaches the transport, is an arm of
#     crates/web/src/dispatch.rs or is listed in apps/web/src/lib/previewOnly.ts.
#  2. No name listed as preview-only has a dispatch arm: a list that outlives
#     the gap it records would refuse a command the box now answers.
#  3. Every name listed as preview-only is answered by mockPreview.ts, so the
#     preview at least does what its controls offer.
#
# It refuses to report OK when it found no dispatch arm, no preview-only list,
# or no command name in the frontend: each would mean a path moved and the
# comparison ran over nothing.
#
# WHAT IT CANNOT DO
#
# It reads quoted names. A command name built at run time (a template literal,
# a name passed through a variable from somewhere else) is not seen; the per
# plane wrappers all take their name as a literal today. It says nothing about
# the arguments, only that the name is handled.
set -uo pipefail
cd "$(git rev-parse --show-toplevel)" || exit 1

dispatch="crates/web/src/dispatch.rs"
list="apps/web/src/lib/previewOnly.ts"
mock="apps/web/src/lib/mockPreview.ts"
for f in "$dispatch" "$list" "$mock"; do
	[ -f "$f" ] || { echo "measured nothing: $f is not here"; exit 2; }
done

arms=$(grep -oE '^        "[a-z][a-z0-9_]*"( \| "[a-z][a-z0-9_]*")* =>' "$dispatch" |
	grep -oE '"[a-z][a-z0-9_]*"' | tr -d '"' | sort -u)
[ -n "$arms" ] || { echo "measured nothing: no dispatch arm found in $dispatch"; exit 2; }

preview=$(sed -n '/preview-only-commands:begin/,/preview-only-commands:end/p' "$list" |
	grep -oE '^  "[a-z][a-z0-9_]*",$' | grep -oE '[a-z][a-z0-9_]*' | sort -u)
[ -n "$preview" ] || { echo "measured nothing: no preview-only list between the markers in $list"; exit 2; }

# The modules that can reach the box: every non-test module importing the
# transport or the ceremony wrapper. The mock itself is not a sender.
files=$(grep -rlE 'from "(\./|\.\./|\.\./lib/)(transport|webauthn)"' apps/web/src --include='*.ts' --include='*.tsx' |
	grep -vE '\.test\.tsx?$' | grep -vE '/mockPreview\.ts$' | sort)
[ -n "$files" ] || { echo "measured nothing: no module under apps/web/src reaches the transport"; exit 2; }

# Newlines folded so a call whose name sits on the next line is still read.
# A method call (`.getItem("x_y")`) is not a command and is dropped.
names=""
for f in $files; do
	found=$(tr '\n' ' ' <"$f" |
		grep -oE '\.?[A-Za-z_][A-Za-z0-9_]*(<[^>()]*>)?\([[:space:]]*"[a-z][a-z0-9]*(_[a-z0-9]+)+"' |
		grep -vE '^\.' | grep -oE '"[a-z0-9_]+"$' | tr -d '"' || true)
	[ -z "$found" ] || names=$(printf '%s\n%s' "$names" "$found")
done
names=$(printf '%s\n' "$names" | grep -E '^[a-z]' | sort -u)
[ -n "$names" ] || { echo "measured nothing: no command name found in the modules that reach the transport"; exit 2; }

in_set() { grep -qxF -- "$1" <<<"$2"; }

fail=0
sent=0
for n in $names; do
	sent=$((sent + 1))
	if ! in_set "$n" "$arms" && ! in_set "$n" "$preview"; then
		echo "FAIL: the console sends \"$n\", which $dispatch has no arm for and $list does not list as preview-only"
		fail=1
	fi
done

for n in $preview; do
	if in_set "$n" "$arms"; then
		echo "FAIL: \"$n\" is listed preview-only in $list, but $dispatch now handles it: drop it from the list"
		fail=1
	fi
	if ! grep -qF -- "case \"$n\":" "$mock"; then
		echo "FAIL: \"$n\" is listed preview-only in $list, but $mock does not answer it either"
		fail=1
	fi
done

[ "$fail" -eq 0 ] || exit 1
echo "OK: $sent command names sent by the console, each handled by genaryx-web or listed preview-only ($(wc -w <<<"$preview" | tr -d ' ') listed, each answered by the preview)"
