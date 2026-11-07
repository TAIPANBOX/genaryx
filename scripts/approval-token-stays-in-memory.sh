#!/usr/bin/env bash
# The raw approval token stays in the one answer that carries it and in the
# memory of the component that shows it (CLAUDE.md invariant 46).
#
# WHY THIS EXISTS
#
# A grant's `approval_token` is a live credential, and wardryx hands it over
# once. The tests prove that today's code keeps it out of the journal, the logs,
# the browser's storage and Felyx. What they cannot prove is that the NEXT change
# does: a new panel reading the field, a console.log left in a handler, a view
# kept mounted across navigation, a helper that makes the type printable. Each of
# those compiles and passes every test written before it. So this reads the
# places they would be written.
#
# WHAT IT HOLDS
#
# - only the modules named below touch the raw token in the web console, and
#   they reach no browser storage, no URL, no console line and no network call of
#   their own;
# - AppShell mounts the two views that hold a token only while they are the
#   active view, which is what makes leaving a view drop it;
# - nothing in the copilot crate names the token or the call that mints it;
# - the token's type cannot be printed, and has no accessor;
# - in the policy commands, exactly the two functions that have a reason to read
#   the field read it;
# - the answer to a decision tells every cache to keep nothing.
#
# WHAT IT DOES NOT DO
#
# It reads names, not meaning. A module that takes the token under another name
# from a holder would pass here; the jsdom tests and the router tests are what
# watch the bytes.

set -uo pipefail
cd "$(git rev-parse --show-toplevel)" || exit 1

web=apps/web/src
token_module=$web/lib/approvalToken.ts
holders=(
	"$token_module"
	"$web/components/ApprovalsInbox.tsx"
	"$web/components/PolicyView.tsx"
	"$web/components/CopilotView.tsx"
)
wire_field_allowed=("$web/policyTypes.ts" "$token_module" "$web/lib/mockPreview.ts")
shell=$web/components/AppShell.tsx
commands=crates/api/src/policy/commands.rs
connector=crates/connectors/src/wardryx.rs
dispatch=crates/web/src/dispatch.rs
copilot=crates/copilot/src

for f in "${holders[@]}" "${wire_field_allowed[@]}" "$shell" "$commands" "$connector" "$dispatch"; do
	if [ ! -f "$f" ]; then
		echo "measured nothing: $f is gone, which is a failure of this script" >&2
		echo "and not a clean bill of health." >&2
		exit 1
	fi
done
if [ ! -d "$copilot" ]; then
	echo "measured nothing: $copilot is gone." >&2
	exit 1
fi

fail=0
note() {
	printf '%s\n' "$1"
	fail=$((fail + 1))
}

is_one_of() {
	local needle="$1"
	shift
	local item
	for item in "$@"; do
		[ "$item" = "$needle" ] && return 0
	done
	return 1
}

# The product half of a Rust file: its tests may build whatever they like.
product() { sed '/^mod tests {/,$d' "$1"; }

# Product modules of the console, tests left out.
web_modules=$(git ls-files "$web" | grep -E '\.tsx?$' | grep -vE '\.test\.tsx?$')
if [ -z "$web_modules" ]; then
	echo "measured nothing: git lists no module under $web." >&2
	exit 1
fi

# 1. The wire field, and the type that holds it, are named where they are allowed.
holders_seen=0
while IFS= read -r f; do
	if grep -q 'approval_token' "$f" && ! is_one_of "$f" "${wire_field_allowed[@]}"; then
		note "$f names approval_token: only ${wire_field_allowed[*]} may read the wire field"
	fi
	if grep -qE '\bGrantedToken\b|GrantedTokenPanel|grantedTokenFrom' "$f"; then
		holders_seen=$((holders_seen + 1))
		is_one_of "$f" "${holders[@]}" ||
			note "$f holds a granted token: add it to this gate on purpose, or do not hold one"
	fi
done <<<"$web_modules"
if [ "$holders_seen" -eq 0 ]; then
	echo "measured nothing: no module holds a granted token, so the names this reads have moved." >&2
	exit 1
fi

# 2. A module that holds a token reaches nothing that outlives it.
leaks='localStorage|sessionStorage|indexedDB|document\.cookie|history\.(pushState|replaceState)|location\.(href|hash|search|pathname)[[:space:]]*=|location\.(assign|replace)\(|sendBeacon|postMessage|console\.(log|info|warn|error|debug|trace)\(|[^.[:alnum:]_]fetch\('
for f in "${holders[@]}"; do
	found=$(grep -nE "$leaks" "$f")
	if [ -n "$found" ]; then
		note "$f holds a token and reaches past this component:"
		note "$found"
	fi
done

# 3. Leaving a view drops what it held.
for pair in policy:PolicyView copilot:CopilotView; do
	view=${pair%%:*}
	component=${pair##*:}
	mounted=$(grep -A1 -F "{view === \"$view\" && (" "$shell")
	grep -q "<$component" <<<"$mounted" ||
		note "$shell does not mount $component only while it is the active view, so leaving it would keep its tokens"
done

# 4. Felyx has no road to a token.
named=$(grep -rnE 'approval_token|decide_approval|ApprovalDecideResponse|RawApprovalToken|DecideOutcome' "$copilot")
if [ -n "$named" ]; then
	note "the copilot crate names the token or the call that mints it:"
	note "$named"
fi

# 5. The token's types cannot be printed.
if ! grep -q 'pub struct RawApprovalToken(String);' "$commands"; then
	note "$commands: RawApprovalToken is no longer a private tuple field over a String"
fi
if grep -q 'derive(.*Debug' <<<"$(grep -B3 'pub struct RawApprovalToken' "$commands")"; then
	note "$commands: RawApprovalToken derives Debug, which prints the token"
fi
if grep -q 'derive(.*Debug' <<<"$(grep -B3 'pub struct ApprovalDecideResponse' "$connector")"; then
	note "$connector: ApprovalDecideResponse derives Debug, which prints the token"
fi
if grep -qE 'Display for RawApprovalToken|impl RawApprovalToken' "$commands"; then
	note "$commands: RawApprovalToken has a way to be read; it has only its Debug and its serialised form"
fi

# 6. Two readers, each with a reason: the claims for the journal, the raw token
# for the answer. A third is a decision to make here, on purpose.
readers=$(product "$commands" | grep -c '\.approval_token')
if [ "$readers" -ne 2 ]; then
	note "$commands reads .approval_token $readers times outside its tests: describe_decision_result and raw_token_of are the two with a reason"
fi

# 7. Nothing between here and the browser keeps the answer.
if ! grep -q 'Ok(no_store(reply(' <<<"$(grep -A12 '"policy_decide_approval" =>' "$dispatch")"; then
	note "$dispatch: the answer to a decision is not marked no-store"
fi

if [ "$fail" -gt 0 ]; then
	printf '\nthe raw approval token is a live credential with one copy: invariant 46.\n'
	exit 1
fi
printf 'OK: the approval token stays in its answer and in component memory.\n'
