#!/usr/bin/env bash
# What a person is shown of a held call's arguments stays in the answer that
# carries it and in the row that shows it (CLAUDE.md invariant 48).
#
# WHY THIS EXISTS
#
# The preview is a held call's arguments with secrets hidden: still customer
# data. The tests prove today's code keeps it out of the journal and renders it
# as text. What they cannot prove is that the NEXT change does: a journal line
# that records the document, Felyx handed the preview to explain a hold, a
# component that writes it to browser storage, a renderer that takes HTML. Each
# compiles and passes every test written before it. So this reads the code for
# those shapes.
#
# WHAT IT HOLDS
#
#  1. The command that opens a preview journals nothing: no `journal(`, no
#     `command::record`, no `CommandRecord` in crates/api/src/policy/preview.rs.
#  2. Felyx (crates/copilot) never names the preview command, the preview key
#     or the preview type.
#  3. The browser files that hold a preview write no browser storage and render
#     no HTML: no localStorage, sessionStorage, indexedDB or
#     dangerouslySetInnerHTML in ArgumentPreview.tsx or ApprovalsInbox.tsx.
#  4. Only the inbox and the preview component name the preview type in the web
#     app, apart from its declaration and the transport.
#
# WHAT IT CANNOT DO
#
# It reads shapes. A copy made through a helper with another name needs a
# reader; the tests are that reader for the paths that exist.
set -uo pipefail
cd "$(git rev-parse --show-toplevel)" || exit 1

api="crates/api/src/policy/preview.rs"
web_preview="apps/web/src/components/ArgumentPreview.tsx"
web_inbox="apps/web/src/components/ApprovalsInbox.tsx"
for f in "$api" "$web_preview" "$web_inbox"; do
	[ -f "$f" ] || { echo "measured nothing: $f is not here"; exit 2; }
done
[ -d crates/copilot/src ] || { echo "measured nothing: crates/copilot/src is not here"; exit 2; }

fail=0

if grep -nE 'journal\(|command::record|CommandRecord' "$api" >/dev/null; then
	echo "FAIL: $api journals: $(grep -nE 'journal\(|command::record|CommandRecord' "$api" | head -2 | tr '\n' ' ')"
	fail=1
fi

if grep -rnE 'policy_approval_preview|PreviewKey|PreviewDto|SeenPreviews' crates/copilot/src >/dev/null; then
	echo "FAIL: Felyx names the preview: $(grep -rlE 'policy_approval_preview|PreviewKey|PreviewDto|SeenPreviews' crates/copilot/src | tr '\n' ' ')"
	fail=1
fi

for f in "$web_preview" "$web_inbox"; do
	if grep -nE 'localStorage|sessionStorage|indexedDB|dangerouslySetInnerHTML' "$f" >/dev/null; then
		echo "FAIL: $f writes browser storage or renders HTML: $(grep -nE 'localStorage|sessionStorage|indexedDB|dangerouslySetInnerHTML' "$f" | head -2 | tr '\n' ' ')"
		fail=1
	fi
done

holders=$(grep -rlE '\bApprovalPreview\b|\bPreviewDocument\b' apps/web/src --include='*.ts' --include='*.tsx' |
	grep -vE '\.test\.tsx?$' |
	grep -vE '^apps/web/src/(policyTypes\.ts|lib/policy\.ts|components/ArgumentPreview\.tsx|components/ApprovalsInbox\.tsx)$' || true)
if [ -n "$holders" ]; then
	echo "FAIL: a module outside the list holds a preview: $holders"
	fail=1
fi

[ "$fail" -eq 0 ] || exit 1
echo "OK: the preview is journaled nowhere, never reaches Felyx, and the two files that hold it keep no storage and render no HTML"
