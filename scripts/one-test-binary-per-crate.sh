#!/usr/bin/env bash
# Every crate links its integration tests into ONE test binary.
#
# WHY
#
# Cargo makes every top-level `tests/*.rs` file its own executable, and every
# one of them statically links the crate and its whole dependency tree. So one
# `cargo test` after a change relinks all of them, and the bytes written are
# the size of one test binary times the number of files. Measured on a sibling
# repo in this estate: about forty such binaries of 200 MB each, about 8 GB
# written per run, and several TB to one laptop's SSD in a week of ordinary
# work. The fix is one binary per crate: `tests/it/main.rs` declares each file
# as a module, and the files move under `tests/it/` with their test names
# unchanged.
#
# The failure this guards against is the easy one: somebody adds a new
# `tests/foo.rs` next to `tests/it/`, it works, and the cost comes back one file
# at a time with nothing visibly wrong.
#
# WHAT COUNTS
#
# A crate is a directory with a Cargo.toml holding a `[package]`, anywhere in
# the repo outside build output. A test binary is a top-level `tests/*.rs` file
# (cargo's auto-discovery; `tests/it/` and `tests/fixtures/` are not top level),
# or a `[[test]]` table in the crate's Cargo.toml naming anything other than
# `it`. More than one in a crate fails, unless the extra one is in the
# allow-list below with its reason.

set -uo pipefail
cd "$(git rev-parse --show-toplevel)" || exit 1

# Binaries kept apart on purpose. One per line: the path, then the reason. An
# entry whose file no longer exists fails too, so the list cannot quietly
# outlive what it excuses.
ALLOWED=(
	# Sets HTTP_PROXY/ALL_PROXY for the whole process to prove a gated client
	# ignores them; inside the shared binary every other reqwest client in the
	# crate's tests would route through that proxy while it is set.
	"crates/copilot/tests/residency_no_proxy_test.rs"
)

allowed() {
	local p
	for p in "${ALLOWED[@]}"; do [ "$p" = "$1" ] && return 0; done
	return 1
}

fail=0
crates=0
binaries=0

for p in "${ALLOWED[@]}"; do
	if [ ! -f "$p" ]; then
		printf 'STALE     %s\n          the allow-list names a file that does not exist\n' "$p"
		fail=$((fail + 1))
	fi
done

while IFS= read -r manifest; do
	grep -q '^\[package\]' "$manifest" || continue
	crates=$((crates + 1))
	dir="${manifest%/Cargo.toml}"
	counted=()
	if [ -d "$dir/tests" ]; then
		while IFS= read -r f; do
			binaries=$((binaries + 1))
			allowed "$f" || counted+=("$f")
		done < <(find "$dir/tests" -maxdepth 1 -type f -name '*.rs' | sort)
		[ -f "$dir/tests/it/main.rs" ] && counted+=("$dir/tests/it/main.rs") &&
			binaries=$((binaries + 1))
	fi
	while IFS= read -r name; do
		binaries=$((binaries + 1))
		counted+=("$manifest [[test]] $name")
	done < <(awk '/^\[\[test\]\]/{t=1; next} /^\[/{t=0} t && /^name *=/{gsub(/[" ]/,""); sub(/name=/,""); if ($0 != "it") print}' "$manifest")
	if [ "${#counted[@]}" -gt 1 ]; then
		printf 'SPLIT     %s has more than one integration-test binary:\n' "$dir"
		printf '          %s\n' "${counted[@]}"
		printf '          move the files under tests/it/ and declare each in tests/it/main.rs,\n'
		printf '          or allow-list one in %s with the reason it must run alone\n' "$0"
		fail=$((fail + 1))
	fi
done < <(find . -name Cargo.toml -not -path '*/target/*' -not -path '*/node_modules/*' -not -path './.git/*' | sed 's|^\./||' | sort)

if [ "$crates" -eq 0 ] || [ "$binaries" -eq 0 ]; then
	echo "measured nothing: found $crates crate(s) and $binaries test binary(ies), which" >&2
	echo "is a failure of this script and not a clean bill of health." >&2
	exit 1
fi

printf 'test binaries: %d crate(s), %d integration-test binary(ies), %d allow-listed, %d problem(s)\n' \
	"$crates" "$binaries" "${#ALLOWED[@]}" "$fail"
[ "$fail" -eq 0 ]
