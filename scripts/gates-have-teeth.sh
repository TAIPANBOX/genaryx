#!/usr/bin/env bash
# Checks that the gates in `scripts/` still FAIL on the faults they exist to
# catch, still PASS on what they must not catch, and REFUSE to report success
# when they measured nothing at all.
#
# WHY
#
# Every gate here parses text, and a text parser does not break loudly: it
# stops matching and reports success. The mutants that proved each one existed
# as prose, in commit messages and in the `*(gate: ...)*` markers in CLAUDE.md,
# which is a record of what was true once. Nothing ran them again.
#
# A gate that has quietly stopped catching anything looks exactly like a gate
# with nothing to catch, and stays that way until the fault it guards ships.
#
# WHY THE THIRD PROPERTY IS SEPARATE FROM THE FIRST
#
# `readme-numbers.sh` says in its own words that a suite reporting no tests
# means it measured nothing. That sentence was true and had never been re-run.
#
# It also had a nearer relative of the same fault, fixed in the commits before
# this one: it took the test count from a run whose exit code it discarded, so
# ONE failing test truncated the workspace from six crates to two, the sum fell
# from 688 to 479, and it reported that the README was lying. The README was
# correct. A number read from a broken run is not a smaller number, it is a
# different measurement wearing the same units.
#
# The three grep-shaped gates are the other risk: a pattern that stops matching
# reports success, and every one of them prints OK from an empty result set.
#
# HOW IT MUTATES WITHOUT LEAVING A MESS
#
# It edits tracked files in place, so it refuses to start unless the tree is
# clean, restores with `git checkout` after every case, restores again from a
# trap on any exit path including a kill, and asserts the tree is clean before
# reporting success.
#
#
# A GATE THAT IS ALREADY FAILING CANNOT BE JUDGED
#
# No case proves anything if the gate was already failing before the mutation.
# So every case runs the gate on the UNMUTATED tree first and reports
# UNJUDGEABLE. Found on 2026-08-09 in it-rat, where one gate was legitimately
# red and a case against it would have been indistinguishable from a working
# one.
#
# It covered only the fail-cases at first, which left the mirror of the same
# bug: on a red gate a pass-case reports OVEREAGER, "the gate failed on
# something it must not catch", and sends the reader to look at a harmless
# mutation. The verdict was being given without the predicate it depends on.
#
# A MUTATION THAT DID NOT APPLY PROVES NOTHING
#
# Every edit asserts it changed the file. A case whose edit applied nothing is
# a failure here, not a pass. That is not hypothetical: five such mutations
# were caught across idryx and tokenfuse on 2026-08-09, and three of the five
# had been verified BY HAND against the same gate minutes earlier. The hand
# version and the harness version differ only in how many layers of quoting sit
# between the text and python, which is exactly the difference nobody sees.

set -uo pipefail
cd "$(git rev-parse --show-toplevel)" || exit 1

if [ -n "$(git status --porcelain)" ]; then
	printf 'this script mutates tracked files, so it needs a clean tree.\n'
	printf 'commit or stash first; it restores with `git checkout` and cannot\n'
	printf 'tell your edits from its own.\n'
	exit 1
fi

# Untracked files too: a mutation may RENAME a tracked file, and `git checkout`
# restores the original while leaving the new name behind. And the INDEX, since
# a gate may read `git ls-files` rather than the disk, so a mutation has to move
# the file in both. Safe because this
# script refuses to start unless the tree is clean, so anything untracked
# during a run was created by the run. `-x` is deliberately absent: ignored
# build output is not ours to delete.
restore() {
	git reset -q --hard HEAD 2>/dev/null
	git clean -fdq 2>/dev/null
}
baseline_dir="$(mktemp -d)"

# One trap for both, because a second `trap ... EXIT` REPLACES the first
# rather than adding to it. Writing them separately disarmed `restore` on
# every interrupt path, which would leave a mutated tree behind on Ctrl-C.
cleanup() {
	restore
	rm -rf "$baseline_dir"
}
trap cleanup EXIT INT TERM


failures=0
cases=0

# run_case <name> <expect: fail|pass> <gate> <python edit> [required output]
#
# The needle separates "it failed" from "it failed for the reason this case is
# about". Without it, a case expecting failure is satisfied by any failure,
# including one this harness caused itself.
run_case() {
	local name="$1" expect="$2" gate="$3" edit="$4" needle="${5:-}"
	cases=$((cases + 1))

	# The baseline applies to EVERY case, not only the ones expecting a failure.
	# It was `fail`-only until 2026-08-09, which left the mirror of the bug it was
	# written for: on a gate that is already red, a `pass` case reports OVEREAGER,
	# "the gate failed on something it must not catch", and sends the reader to
	# look at a harmless mutation while the gate was failing without it. Neither
	# verdict means anything on a red gate, so neither is given.
	skip_baseline=0
	if [ "$expect" = fail_env ]; then
		# `fail` with the baseline skipped, for cases whose fault IS the command
		# rather than a mutation: red before and after is the point there.
		expect=fail
		skip_baseline=1
	fi

	if [ "$skip_baseline" = 0 ]; then
		local key base_out
		key="$baseline_dir/$(printf '%s' "$gate" | cksum | tr -d ' ')"
		if [ ! -f "$key" ]; then
			if eval "$gate" >/dev/null 2>&1; then printf 'green' >"$key"; else printf 'red' >"$key"; fi
		fi
		base_out="$(cat "$key")"
		if [ "$base_out" = red ]; then
			printf 'UNJUDGEABLE  %s\n             the gate is already failing on a clean tree, so neither a\n             failure nor a pass after the mutation would prove anything\n' "$name"
			failures=$((failures + 1))
			return
		fi
	fi

	if ! python3 -c "$edit"; then
		printf 'BROKEN  %s\n        its mutation did not apply, so this case proved nothing\n' "$name"
		failures=$((failures + 1))
		restore
		return
	fi

	local out rc
	out=$(eval "$gate" 2>&1)
	rc=$?
	restore

	# Exit code first, then wording. Checking the needle before the expectation
	# turns "it did not fail at all" into "it failed for the wrong reason",
	# which sends the reader to look at prose when the gate is toothless.
	if [ "$expect" = fail ] && [ "$rc" -ne 0 ] && [ -n "$needle" ] &&
		! printf '%s' "$out" | grep -qF -- "$needle"; then
		printf 'WRONG REASON  %s\n              it failed, but not saying: %s\n' "$name" "$needle"
		failures=$((failures + 1))
		return
	fi
	if [ "$expect" = fail ] && [ "$rc" -eq 0 ]; then
		printf 'TOOTHLESS  %s\n           the gate passed on a fault it exists to catch\n' "$name"
		failures=$((failures + 1))
	elif [ "$expect" = pass ] && [ "$rc" -ne 0 ]; then
		printf 'OVEREAGER  %s\n           the gate failed on something it must not catch\n' "$name"
		failures=$((failures + 1))
		printf '%s\n' "$out" | head -4 | sed 's/^/           /'
	else
		printf 'ok  %-58s (%s)\n' "$name" "$expect"
	fi
}

py() { printf 'def edit(p, a, b):\n    s = open(p).read()\n    assert a in s, "pattern not found in " + p\n    open(p, "w").write(s.replace(a, b, 1))\n%s\n' "$1"; }

echo "=== faults each gate must catch ==="

# invariant 1: the console holds no cloud key. The provider list is the point
# of failure rather than the grep, and Hetzner was missing from it until
# 2026-08-06 while a Hetzner connector shipped.
run_case "no-cloud-credentials: a cloud key read from the environment" fail \
	'./scripts/no-cloud-credentials.sh' \
	"$(py 'edit("crates/api/src/events.rs", "use ", "// a token read at runtime is exactly what this gate exists to refuse\nfn _teeth() -> String { std::env::var(\"HCLOUD_TOKEN\").unwrap_or_default() }\n\nuse ")')" \
	"Reading a cloud credential"

run_case "no-cloud-credentials: a provider SDK declared as a dependency" fail \
	'./scripts/no-cloud-credentials.sh' \
	"$(py 'edit("crates/api/Cargo.toml", "[dependencies]", "[dependencies]\naws-sdk-s3 = \"1\"")')" \
	"declares"

# invariant: fixture rows reach no panel. A module importing them outside the
# one allowed file is how a demo stream becomes a dashboard.
run_case "no-fabricated-rows: a panel imports the fixture stream" fail \
	'./scripts/no-fabricated-rows.sh' \
	"$(py 'edit("apps/web/src/admissionTypes.ts", "export ", "import { events } from \"./lib/mockData\";\nexport const _teeth = events;\n\nexport ")')" \
	"imports fixture rows from mockData"

# invariants 5 and 6: web only, and no purchase surface. Both were executed
# once and both grow back one convenience at a time.
run_case "web-only-and-unpriced: a desktop shell dependency returns" fail \
	'./scripts/web-only-and-unpriced.sh' \
	"$(py 'import json
p = "apps/web/package.json"
d = json.load(open(p))
d.setdefault("dependencies", {})["tauri"] = "^2"
json.dump(d, open(p, "w"), indent=2)')" \
	"FAIL"

echo
echo "=== and what they must NOT catch ==="

# The one file allowed to import fixtures. A gate that flagged it would be
# flagging the design it protects.
run_case "no-fabricated-rows: the one module allowed to import fixtures" pass \
	'./scripts/no-fabricated-rows.sh' \
	"$(py 'p = "apps/web/src/lib/recentEvents.ts"
s = open(p).read()
open(p, "w").write("// a harmless comment added beside the allowed import\n" + s)')"

echo
echo "=== and the one this estate learned the hard way ==="
echo "    a gate whose subject is gone must SAY so, not report OK on nothing"

# The gate now reads the exit code rather than discarding it, so a broken suite
# is reported as a broken suite instead of as a lying README.
run_case "readme-numbers: a failing suite is not a smaller count" fail \
	'./scripts/readme-numbers.sh' \
	"$(py 'edit("crates/core/src/bus.rs", "#[cfg(test)]", "#[cfg(test)]\nmod teeth_forced_failure {\n    #[test]\n    fn this_test_fails_on_purpose() { assert!(false, \"planted by gates-have-teeth\"); }\n}\n\n#[cfg(test)]")')" \
	"the suite did not pass"

echo
if [ -n "$(git status --porcelain)" ]; then
	printf 'FAIL: this script left the tree dirty, so it cannot be trusted about anything above\n'
	git status --porcelain | head -5
	exit 1
fi

if [ "$failures" -gt 0 ]; then
	printf '%d of %d cases failed.\n' "$failures" "$cases"
	printf 'A gate that has quietly stopped catching anything looks exactly like a gate\n'
	printf 'with nothing to catch, and stays that way until the fault it guards ships.\n'
	exit 1
fi

printf 'OK: %d cases. Every gate fails on its own fault, passes on a non-fault,\n' "$cases"
printf '    and refuses to report success when it measured nothing.\n'
