#!/usr/bin/env bash
# The Trailryx crates the console decodes the record with are the release
# whose binaries the image carries (CLAUDE.md invariant 54).
#
# WHY THIS EXISTS
#
# The Record tab reads sealed manifests with trailryx's own decoder, pinned by
# commit in crates/api/Cargo.toml, and verifies with trailryx-node and
# trailryx-verify, copied into the image from ghcr.io/taipanbox/trailryx-node
# at a tag. Move one without the other and the console shows a root computed by
# one release beside a verdict from another, and every check is green.
#
# WHAT IT HOLDS
#
#  1. Every trailryx git dependency in crates/api/Cargo.toml names one and the
#     same `rev`.
#  2. The Dockerfile copies the binaries from exactly one trailryx-node tag,
#     pinned by digest, and labels the same reference.
#  3. The `rev` is the commit that tag names, as the comment table below
#     records it; a new pin must add its row.
#
# It refuses to report OK when it finds no trailryx dependency or no
# trailryx-node image.
set -uo pipefail
cd "$(git rev-parse --show-toplevel)" || exit 1

cargo_toml="crates/api/Cargo.toml"
dockerfile="Dockerfile"
for f in "$cargo_toml" "$dockerfile"; do
	[ -f "$f" ] || { echo "measured nothing: $f is not here"; exit 2; }
done

# The release each pinned commit is, read with `git ls-remote --tags` (the
# peeled commit). One row per pin this repository has used.
known="cc3d2e8a7c49bf6843b735461032c6fc2c17df3b v1.6.0"

revs=$(grep -E '^trailryx-[a-z]+ = \{ git = "https://github.com/TAIPANBOX/trailryx"' "$cargo_toml" |
	grep -oE 'rev = "[0-9a-f]{40}"' | grep -oE '[0-9a-f]{40}' | sort -u)
[ -n "$revs" ] || { echo "measured nothing: no trailryx git dependency in $cargo_toml"; exit 2; }

tags=$(grep -oE '^FROM ghcr\.io/taipanbox/trailryx-node:v[0-9.]+@sha256:[0-9a-f]{64}' "$dockerfile" |
	grep -oE ':v[0-9.]+@' | tr -d ':@' | sort -u)
[ -n "$tags" ] || { echo "measured nothing: no trailryx-node image pinned by digest in $dockerfile"; exit 2; }

fail=0
if [ "$(wc -l <<<"$revs" | tr -d ' ')" -ne 1 ]; then
	echo "FAIL: the trailryx crates in $cargo_toml name more than one rev: $(tr '\n' ' ' <<<"$revs")"
	fail=1
fi
if [ "$(wc -l <<<"$tags" | tr -d ' ')" -ne 1 ]; then
	echo "FAIL: $dockerfile pins more than one trailryx-node tag: $(tr '\n' ' ' <<<"$tags")"
	fail=1
fi
rev=$(head -1 <<<"$revs")
tag=$(head -1 <<<"$tags")
named=$(grep -E "^$rev " <<<"$known" | awk '{print $2}')
if [ -z "$named" ]; then
	echo "FAIL: rev $rev is not in this gate's table of trailryx releases; add the row it is"
	fail=1
elif [ "$named" != "$tag" ]; then
	echo "FAIL: the crates are trailryx $named (rev $rev) but the image copies trailryx-node $tag"
	fail=1
fi
if ! grep -qE "^LABEL dev\.taipanbox\.engine\.trailryx=\"$tag@sha256:" "$dockerfile"; then
	echo "FAIL: $dockerfile does not label the trailryx engine as $tag"
	fail=1
fi

[ "$fail" -eq 0 ] || exit 1
echo "OK: the record is decoded with trailryx $tag's crates and verified with trailryx $tag's binaries"
