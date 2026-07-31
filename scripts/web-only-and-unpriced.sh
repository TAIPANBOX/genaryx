#!/usr/bin/env bash
# Holds invariants 5 and 6 of CLAUDE.md: web-only, and nothing here is paid.
#
# WHY THESE TWO TOGETHER. Both are decisions that were already made and already
# executed once, estate-wide, and both are the kind that grow back one
# convenience at a time. A dependency added for a "quick native build". An
# upsell tile added because the money plane happens to return a 402. Neither
# announces itself, and both are visible to anyone reading a public repository.
#
# HOW INVARIANT 5 IS CHECKED: by artefact, not by vocabulary. A desktop shell
# leaves objects behind, a Tauri config, an Xcode project, Swift sources, a
# dependency in a manifest. Those are unambiguous and checking for them says
# nothing about any product. A word-list check would be worse in both
# directions: it would miss a shell added under another name, and it would flag
# honest history. This repository legitimately records that Tauri and SwiftUI
# shells once existed and were removed, in the PAST tense, and that record is
# worth keeping.
#
# HOW INVARIANT 6 IS CHECKED: in what the console SHOWS, not in what it says
# about itself. The word "upgrade" has an honest meaning here (software-signed
# actions upgrade to hardware-confirmed; an agent is literally named
# dependency-upgrader), so a bare word list would cry wolf and get disabled.
# What must not exist is a rendered surface that asks an operator to buy
# something: an upsell component, or a purchase URL displayed in the UI.
#
# The receiver for the old `plan_required` 402 is deliberately still parsed, so
# a console pointed at an older Cloud reports the refusal honestly instead of
# going blank. Parsing it is fine. Rendering a call to buy is not.
#
# This file is the ONE copy of this check.

set -uo pipefail
cd "$(git rev-parse --show-toplevel)" || exit 1

problems=0
note() {
	echo "FAIL: $1"
	problems=$((problems + 1))
}

# ------------------------------------------------------- invariant 5, by artefact
while IFS= read -r f; do
	[ -n "$f" ] || continue
	case "$f" in
	*tauri.conf.json | *.xcodeproj/* | *.xcworkspace/* | *.swift | *Package.swift | *.entitlements | *Info.plist)
		note "$f is a desktop-shell artefact. This product is web-only: the native shells were removed on 2026-07-24 and are not coming back."
		;;
	esac
done < <(git ls-files)

for manifest in $(git ls-files 'Cargo.toml' '*/Cargo.toml' 'package.json' '*/package.json'); do
	if grep -qiE '^\s*"?(tauri|tauri-build|wry|tao)"?\s*[:=]' "$manifest"; then
		note "$manifest declares a desktop-shell dependency"
	fi
done

# ------------------------------------------------------- invariant 6, by surface
# A component whose job is to sell something.
while IFS= read -r f; do
	[ -n "$f" ] || continue
	case "$(basename "$f")" in
	*[Uu]psell* | *[Pp]ricing* | *[Pp]aywall* | *[Pp]lans*)
		note "$f is a purchase surface. Nothing in this stack is sold, so a console that shows one is describing a product that does not exist."
		;;
	esac
done < <(git ls-files 'apps/web/src/*')

# A purchase URL reaching the screen.
#
# The boundary is the directory, not the syntax, and the first version of this
# check got that wrong. It looked for `upgrade_url` inside braces, meaning to
# find JSX, and matched the TypeScript type declaration instead: `{ kind:
# "plan_required"; ...; upgrade_url: string }` is braces too. A matcher that
# cannot tell a type from a template should not be deciding.
#
# So: components render, and must not name it. Types and lib code parse, and
# may. That is the actual rule, and it is checkable without guessing at syntax.
for f in $(git ls-files 'apps/web/src/components/*'); do
	if grep -q 'upgrade_url' "$f" 2>/dev/null; then
		note "$f names upgrade_url in a component. The field may be parsed in moneyTypes/lib for compatibility with an older Cloud; putting it on screen asks an operator to buy something."
	fi
done

if [ "$problems" -ne 0 ]; then
	echo
	echo "Web-only and unpriced are decisions that were executed once already."
	echo "Both grow back one convenience at a time. See CLAUDE.md invariants 5 and 6."
	exit 1
fi

echo "OK: no desktop-shell artefact or dependency, and no purchase surface in the console."
