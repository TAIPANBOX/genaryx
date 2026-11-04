#!/usr/bin/env bash
# Every path by which a plane's text can reach Felyx's model, held by name.
#
# WHY THIS EXISTS
#
# A tool result cannot reach the model unfenced: `Message::tool_result` takes a
# `FencedResult`, so that path does not compile. What the type cannot see is a
# plane's text arriving some other way. The one-line summary of an event went
# out as a bare user message until invariant 38, and a read tool's result could
# be handed over through the console's own constructor, which fences nothing.
# Both still compile. So this reads the two places they would be written.
#
# WHAT IT HOLDS
#
# - `Message::tool_result` takes a `FencedResult` and nothing else;
# - every `Message::user(` in the copilot crate's source is one of the three
#   that are allowed: the operator's question, the console's revision request,
#   a text through `fence_text`;
# - `FencedResult::console(` appears once, in the propose path, where what is
#   said back is the console's own;
# - a read tool's result goes through `FencedResult::plane(`;
# - no tool module builds a `Message`.
#
# WHAT IT DOES NOT DO
#
# It does not read what the fence does: `crates/copilot/src/untrusted.rs`'s
# tests and the product mutants in `scripts/gates-have-teeth.sh` hold that. A
# text a tool puts into a structured field, in the right shape, stays plain by
# design.

set -uo pipefail
cd "$(git rev-parse --show-toplevel)" || exit 1

agent=crates/copilot/src/agent.rs
provider=crates/copilot/src/provider/mod.rs
tools=crates/copilot/src/tools

for f in "$agent" "$provider"; do
	if [ ! -f "$f" ]; then
		echo "measured nothing: $f is gone, which is a failure of this script" >&2
		echo "and not a clean bill of health." >&2
		exit 1
	fi
done

# The product half of a file: its tests may build whatever they like.
product() { sed '/^mod tests {/,$d' "$1"; }

fail=0
note() {
	printf '%s\n' "$1"
	fail=$((fail + 1))
}

if ! product "$provider" | grep -A4 'pub fn tool_result(' | grep -q 'result: &FencedResult'; then
	note "$provider: Message::tool_result no longer takes a FencedResult"
fi

users=$(product "$agent" | grep -c 'Message::user(')
if [ "$users" -eq 0 ]; then
	echo "measured nothing: no Message::user( in $agent, so this script is" >&2
	echo "reading the wrong thing." >&2
	exit 1
fi
while IFS= read -r line; do
	case "$line" in
	*'Message::user(question)'* | *'Message::user(request)'* | *'Message::user(fence_text('*) ;;
	*) note "$agent: a user message this gate does not know: $line" ;;
	esac
done < <(product "$agent" | grep 'Message::user(')

console=$(product "$agent" | grep -c 'FencedResult::console(')
if [ "$console" -ne 1 ]; then
	note "$agent: FencedResult::console( appears $console times, and the propose path is the one place it belongs"
fi

if ! product "$agent" | grep -q 'FencedResult::plane(&call.name, &value)'; then
	note "$agent: a read tool's result no longer goes through FencedResult::plane"
fi

if grep -rln 'Message::' "$tools" >/dev/null 2>&1; then
	note "$tools: a tool module builds a Message; a tool returns a value and the loop fences it"
fi

if [ "$fail" -gt 0 ]; then
	printf '\n%d way(s) a plane text could reach the model unfenced.\n' "$fail"
	exit 1
fi
printf 'felyx-fences-plane-text: %d user messages, one console path, every read fenced\n' "$users"
