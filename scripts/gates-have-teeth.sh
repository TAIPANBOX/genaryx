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

# invariant 18: one integration-test binary per crate. The way it is lost is a
# new top-level tests/*.rs that works fine and relinks the whole tree again.
run_case "one-test-binary-per-crate: a second top-level tests/*.rs" fail \
	'./scripts/one-test-binary-per-crate.sh' \
	"$(py 'open("crates/core/tests/planted_by_teeth.rs", "w").write("#[test]\nfn planted() {}\n")')" \
	"SPLIT"

run_case "one-test-binary-per-crate: the allow-listed file is gone" fail \
	'./scripts/one-test-binary-per-crate.sh' \
	"$(py 'import os; os.remove("crates/copilot/tests/residency_no_proxy_test.rs")')" \
	"STALE"

# invariant 21: no tracked file quotes the owner or names him as the one who
# decided. The name and the Cyrillic are assembled at run time, so this file
# is not itself one of the files the gate refuses.
owner_py='N = "Y" + "urii"; U = "".join(chr(c) for c in (0x441, 0x43a, 0x430, 0x437, 0x430, 0x432))'

run_case "no-owner-quotes: the old provenance marker returns" fail \
	'./scripts/no-owner-quotes.sh' \
	"$(py "$owner_py"'
edit("crates/web/src/lifecycle.rs", "//! Operator lifecycle blocking", "//! @" + N.lower() + " 2026-07-24\n//! Operator lifecycle blocking")')" \
	"the old provenance marker"

run_case "no-owner-quotes: the owner named as the one who decided" fail \
	'./scripts/no-owner-quotes.sh' \
	"$(py "$owner_py"'
edit("docs/PHASE6.md", "Name: **Felyx**", "Name: **Felyx** (" + N + " decided this).\n\nName: **Felyx**")')" \
	"the owner named as the one who"

run_case "no-owner-quotes: a quote in his language in a comment" fail \
	'./scripts/no-owner-quotes.sh' \
	"$(py "$owner_py"'
edit("crates/web/src/lifecycle.rs", "//! Operator lifecycle blocking", "// \"" + U + "\"\n//! Operator lifecycle blocking")')" \
	"Ukrainian prose"

run_case "no-owner-quotes: a quoted phrase in his language in a doc" fail \
	'./scripts/no-owner-quotes.sh' \
	"$(py "$owner_py"'
edit("docs/PHASE6.md", "Name: **Felyx**", chr(0xAB) + U + chr(0xBB) + "\n\nName: **Felyx**")')" \
	"a quotation in guillemets"

echo
echo "=== and what they must NOT catch ==="

# The owner as the holder of the copyright is not a quote and not a decision.
run_case "no-owner-quotes: the owner as copyright holder" pass \
	'./scripts/no-owner-quotes.sh' \
	"$(py "$owner_py"'
edit("README.md", "\n", "\n\nCopyright 2026 " + N + " K.\n")')"

# Non-Latin text inside a string literal is test data, which hostile-input
# tests need.
run_case "no-owner-quotes: Cyrillic inside a Rust string literal" pass \
	'./scripts/no-owner-quotes.sh' \
	"$(py "$owner_py"'
edit("crates/connectors/src/urlpath.rs", "\"run-%D0%B9\");", "\"run-%D0%B9\");\n        let _planted = \"" + U + "\";")')"

# The one file allowed to import fixtures. A gate that flagged it would be
# flagging the design it protects.
run_case "no-fabricated-rows: the one module allowed to import fixtures" pass \
	'./scripts/no-fabricated-rows.sh' \
	"$(py 'p = "apps/web/src/lib/recentEvents.ts"
s = open(p).read()
open(p, "w").write("// a harmless comment added beside the allowed import\n" + s)')"

# A new file inside tests/it/ is a module of the one binary, which is the
# shape the gate asks for.
run_case "one-test-binary-per-crate: a new module inside tests/it/" pass \
	'./scripts/one-test-binary-per-crate.sh' \
	"$(py 'open("crates/core/tests/it/planted_by_teeth.rs", "w").write("#[test]\nfn planted() {}\n")
edit("crates/core/tests/it/main.rs", "mod store_test;", "mod store_test;\nmod planted_by_teeth;")')"

# The scenarios in features/ stay bound to tests that exist, both ways:
# a binding pointing at a renamed test reads as held and is not.
run_case "features-are-bound: a binding names a test that is gone" fail \
	'./scripts/features-are-bound.sh' \
	"$(py 'edit("features/the-bus-store-lands-where-the-console-can-write.feature", "@test:an_uncreatable_store_names_every_path_it_tried", "@test:an_uncreatable_store_names_every_path_it_tried_renamed")')" \
	"DANGLING"

run_case "features-are-bound: a scenario with nothing behind it" fail \
	'./scripts/features-are-bound.sh' \
	"$(py 'edit("features/the-bus-store-lands-where-the-console-can-write.feature", "  # @test:an_explicit_state_dir_wins_over_a_writable_taipan_home\n", "")')" \
	"UNBOUND"

# A web test binds by its exact title. A renamed title must leave the scenario
# dangling, or a web binding is a pointer nothing checks.
run_case "features-are-bound: a web binding names a test that is gone" fail \
	'./scripts/features-are-bound.sh' \
	"$(py 'edit("apps/web/src/lib/incidents.test.ts", "it(\"an_identity_refusal_names_the_key_and_the_claimed_agent\"", "it(\"an_identity_refusal_names_the_key_renamed\"")')" \
	"DANGLING"

run_case "features-are-bound: the scenarios taken away entirely" fail \
	'./scripts/features-are-bound.sh' \
	"$(py 'import shutil; shutil.rmtree("features")')" \
	"not a pass"

# invariant 19: an identity refusal is filed under its key. These mutate the
# PRODUCT, not a gate script, and the gate is the tests that hold it: each
# mutant puts one of the four places back on the claimed agent, and the named
# test must go red for it. One cargo command for all five, so one baseline.
identity_gate='cargo test -q --no-fail-fast -p genaryx-core -p genaryx-api -p genaryx-connectors identity_refusal'

run_case "invariant 19: the aggregate groups by the envelope again" fail \
	"$identity_gate" \
	"$(py 'edit("crates/core/src/store.rs", "let subject_sql = crate::attribution::FILED_UNDER_SQL;", "let subject_sql = \"agent_id\";")')" \
	"an_identity_refusal_is_grouped_under_its_key_in_the_aggregate"

run_case "invariant 19: the stop list reads the envelope again" fail \
	"$identity_gate" \
	"$(py 'edit("crates/api/src/stats/mod.rs", "e.filed_under == agent_id", "e.agent_id == agent_id")')" \
	"an_identity_refusal_is_not_on_the_claimed_agents_stop_list"

run_case "invariant 19: a refusal with no key falls back to the claim" fail \
	"$identity_gate" \
	"$(py 'edit("crates/core/src/attribution.rs", "None => NO_KEY.to_string(),", "None => agent_id.to_string(),")')" \
	"an_identity_refusal_with_no_readable_key_names_no_agent"

run_case "invariant 19: cost per action groups refused calls under the claim" fail \
	"$identity_gate" \
	"$(py 'q = chr(39); edit("crates/connectors/src/tokenfuse.rs", "when decision = " + q + "identity_mismatch" + q, "when decision = " + q + "never" + q)')" \
	"an_identity_refusal_is_filed_under_its_key_in_cost_per_action"

run_case "invariant 19: the live graph counts a refusal as the claimed agent's activity" fail \
	"$identity_gate" \
	"$(py 'edit("crates/core/src/graph.rs", "let actor = crate::attribution::filed_under(&ev.event_type, &ev.agent_id, ev.data.as_ref());", "let actor = ev.agent_id.clone();")')" \
	"an_identity_refusal_counts_as_the_keys_activity_in_the_graph"

run_case "invariant 19: the stored graph counts a refusal as the claimed agent's activity" fail \
	"$identity_gate" \
	"$(py 'edit("crates/core/src/store.rs", "SELECT {} AS agent_id, on_behalf_of", "SELECT COALESCE(agent_id, {}) AS agent_id, on_behalf_of")')" \
	"an_identity_refusal_counts_as_the_keys_activity_from_the_store"

run_case "invariant 19: the per-model row counts refused calls again" fail \
	"$identity_gate" \
	"$(py 'q = chr(39); edit("crates/connectors/src/tokenfuse.rs", " where decision <> " + q + "identity_mismatch" + q, "")')" \
	"an_identity_refusal_is_not_a_call_of_the_model_it_named"

# The non-fault: the rule's prose reworded. A gate that fired on it would be
# a gate on wording, which gets disabled the first week.
run_case "invariant 19: the rule's own comment reworded" pass \
	"$identity_gate" \
	"$(py 'edit("crates/core/src/attribution.rs", "/// The prefix of a subject that is a credential rather than an agent.", "/// The prefix every credential subject carries, and no agent id does.")')"

# invariant 20: Felyx counts a reasoning model's billed output. The gate is
# the parser's own tests; the mutant reads the completion count alone again.
run_case "invariant 20: output read from completion_tokens alone again" fail \
	'cargo test -q -p genaryx-copilot --lib provider::openai' \
	"$(py 'edit("crates/copilot/src/provider/openai.rs", "Some(total) => completion.max(total.saturating_sub(prompt)),", "Some(_) => completion,")')" \
	"a_reasoning_model_usage_counts_its_reasoning_as_output"

# invariant 22: closing a retained reservation (D8). Product mutants; the gate
# is the tests that hold the ceremony, the token format, and the rule that a
# 200 alone is not a closure.
run_case "invariant 22: money_close_retained dropped from the ceremony list" fail \
	'cargo test -q -p genaryx-web --bin genaryx-web money_close_retained' \
	"$(py 'edit("crates/web/src/roles.rs", "    gate(\"money_close_retained\", Role::Admin, true),", "    gate(\"money_close_retained\", Role::Admin, false),")')" \
	"money_close_retained_without_an_assertion_is_428"

retained_gate='cargo test -q -p genaryx-api --test it retained_close_test'

run_case "invariant 22: the token is signed over another message" fail \
	"$retained_gate" \
	"$(py 'edit("crates/api/src/money/closure.rs", "const MAC_DOMAIN: &str = \"tokenfuse-closure-v1\";", "const MAC_DOMAIN: &str = \"tokenfuse-closure-v2\";")')" \
	"the_closure_token_matches_the_gateway_test_vector"

run_case "invariant 22: a 200 alone counts as closed" fail \
	"$retained_gate" \
	"$(py 'edit("crates/api/src/money/retained.rs", "let confirmed = answer.body.get(\"closed\").and_then(|v| v.as_bool()) == Some(true);", "let confirmed = true;")')" \
	"a_200_that_does_not_confirm_the_closure_is_never_success"

run_case "invariant 22: the closure module's own comment reworded" pass \
	"$retained_gate" \
	"$(py 'edit("crates/api/src/money/closure.rs", "/// The shortest secret accepted: 32 bytes, the HMAC-SHA256 output size, and", "/// The shortest secret this console accepts: 32 bytes, the HMAC-SHA256 output size, and")')"

# invariant 23: an operator set while the console runs signs in with no
# restart, and one already held is not read again. Product mutants; the gate is
# the router-level tests. Every test name the filter matches carries
# `operator_`, the five of invariant 23 and the passkey ones that lean on it.
operator_gate='cargo test -q -p genaryx-web --bin genaryx-web operator_'

run_case "invariant 23: the operator is read once at startup again" fail \
	"$operator_gate" \
	"$(py 'edit("crates/web/src/ctx.rs", "let found = crate::auth::load(&self.cfg.operator_file())?;", "let found = held?;")')" \
	"an_operator_set_after_start_signs_in_without_a_restart"

run_case "invariant 23: a held operator is read again on every call" fail \
	"$operator_gate" \
	"$(py 'edit("crates/web/src/ctx.rs", "if held.is_some() {", "if held.is_some() && false {")
edit("crates/web/src/ctx.rs", ".get_or_insert(found)", ".insert(found)")')" \
	"an_operator_loaded_at_startup_is_not_reread_from_disk"

run_case "invariant 23: the accessor's own comment reworded" pass \
	"$operator_gate" \
	"$(py 'edit("crates/web/src/ctx.rs", "/// The operator record, read from disk again for as long as there is none.", "/// The operator record, read from disk again while none is held.")')"

# invariant 24: Felyx presents its own gateway client key as x-fuse-key, an
# empty setting sends none, and the key is never shown. Product mutants; the
# gate is the tests that hold it, each of them named with `fuse_key`.
fuse_key_gate='cargo test -q --no-fail-fast -p genaryx-copilot fuse_key'

run_case "invariant 24: the Anthropic client sends no x-fuse-key" fail \
	"$fuse_key_gate" \
	"$(py 'edit("crates/copilot/src/provider/anthropic.rs", "request = request.header(FUSE_KEY_HEADER, key.header_value());", "request = request.header(\"x-fuse-key-planted\", key.header_value());")')" \
	"anthropic_request_carries_the_configured_x_fuse_key_header"

run_case "invariant 24: the OpenAI-compatible client sends no x-fuse-key" fail \
	"$fuse_key_gate" \
	"$(py 'edit("crates/copilot/src/provider/openai.rs", "request = request.header(FUSE_KEY_HEADER, key.header_value());", "request = request.header(\"x-fuse-key-planted\", key.header_value());")')" \
	"openai_compat_request_carries_the_configured_x_fuse_key_header"

run_case "invariant 24: the configured key is read and then dropped" fail \
	"$fuse_key_gate" \
	"$(py 'edit("crates/copilot/src/config.rs", "FuseKey::new(value)\n            .map(Some)", "FuseKey::new(value)\n            .map(|_| None)")')" \
	"felyx_reads_its_fuse_key_file_and_sends_it_through_the_service"

run_case "invariant 24: an empty key is refused instead of sending nothing" fail \
	"$fuse_key_gate" \
	"$(py 'edit("crates/copilot/src/config.rs", "            return Ok(None);\n        }\n        FuseKey::new(value)", "            return Err(ConfigError::BadFuseKey(\"it is empty\"));\n        }\n        FuseKey::new(value)")')" \
	"resolve_fuse_key_unset_empty_file_env_and_refusals"

run_case "invariant 24: an echoed key reaches the error unredacted" fail \
	"$fuse_key_gate" \
	"$(py 'edit("crates/copilot/src/provider/mod.rs", "text.replace(key.as_str(), FUSE_KEY_REDACTED)", "text.replace(FUSE_KEY_REDACTED, key.as_str())")')" \
	"a_gateway_that_echoes_the_fuse_key_never_gets_it_into_felyxs_error"

run_case "invariant 24: a key with a space is accepted" fail \
	"$fuse_key_gate" \
	"$(py 'edit("crates/copilot/src/provider/mod.rs", "if !value.bytes().all(|b| b.is_ascii_graphic()) {", "if value.bytes().any(|b| b == 0) {")')" \
	"a_fuse_key_that_cannot_be_a_header_is_refused_without_repeating_it"

run_case "invariant 24: the header value is not marked sensitive" fail \
	"$fuse_key_gate" \
	"$(py 'edit("crates/copilot/src/provider/mod.rs", "header.set_sensitive(true);", "header.set_sensitive(false);")')" \
	"a_fuse_key_is_sensitive_and_never_printed"

run_case "invariant 24: the redaction marker's own comment reworded" pass \
	"$fuse_key_gate" \
	"$(py 'edit("crates/copilot/src/provider/mod.rs", "/// What stands in for the client key wherever error text would carry it.", "/// The text that replaces the client key wherever an error would carry it.")')"

# invariant 24, the provider key's own reference: a key pasted where
# GENARYX_COPILOT_API_KEY_REF belongs is refused without being repeated. The
# mutant puts the echo back by routing the refusal through an error that already
# carries text, so it compiles and the named test must go red.
api_key_ref_gate='cargo test -q --no-fail-fast -p genaryx-copilot an_api_key_ref_that_holds'

run_case "api_key_ref: the pasted key echoed back into the refusal" fail \
	"$api_key_ref_gate" \
	"$(py 'edit("crates/copilot/src/config.rs", ".map_err(|_| ConfigError::BadApiKeyRef)?", ".map_err(|_| ConfigError::SecretUnavailable(format!(\"api_key_ref must be env:VAR or file:/path, got {reference}\")))?")')" \
	"an_api_key_ref_that_holds_the_key_is_refused_without_repeating_it"

run_case "api_key_ref: the resolution's own comment reworded" pass \
	"$api_key_ref_gate" \
	"$(py 'edit("crates/copilot/src/config.rs", "/// Resolve `api_key_ref` to the secret value, or `None` if unset. Never logs", "/// Resolve `api_key_ref` to the secret value, or `None` when it is unset. Never logs")')"

# invariant 26: a unit's month cap is set by an admin, after a fresh passkey
# assertion, with a reason, and an unread cap map is never "no cap". Product
# mutants; the gate is the tests that hold each half.
run_case "invariant 26: money_set_unit_budget dropped from the ceremony list" fail \
	'cargo test -q -p genaryx-web --bin genaryx-web money_set_unit_budget' \
	"$(py 'edit("crates/web/src/roles.rs", "    gate(\"money_set_unit_budget\", Role::Admin, true),", "    gate(\"money_set_unit_budget\", Role::Admin, false),")')" \
	"money_set_unit_budget_without_an_assertion_is_428"

run_case "invariant 26: setting a unit cap becomes a viewer read" fail \
	'cargo test -q -p genaryx-web --bin genaryx-web unit_cap' \
	"$(py 'edit("crates/web/src/roles.rs", "    gate(\"money_set_unit_budget\", Role::Admin, true),", "    gate(\"money_set_unit_budget\", Role::Viewer, true),")')" \
	"setting_a_unit_cap_is_admin_and_reading_caps_is_a_read"

units_gate='cargo test -q -p genaryx-api --test it money_units_test'

run_case "invariant 26: a cap change with no reason reaches the Cloud" fail \
	"$units_gate" \
	"$(py 'edit("crates/api/src/money/units.rs", "    require_break_glass_reason(&reason)?;\n    validate_unit_cap(&unit, budget_usd)?;", "    validate_unit_cap(&unit, budget_usd)?;")')" \
	"a_unit_cap_change_that_cannot_be_a_cap_never_reaches_the_cloud"

run_case "invariant 26: an unread cap map reads as no cap" fail \
	"$units_gate" \
	"$(py 'edit("crates/api/src/money/units.rs", "            UnitCapState::CapUnknown\n        };", "            UnitCapState::NoCap\n        };")')" \
	"a_cloud_without_unit_budgets_shows_caps_unknown_never_no_cap"

run_case "invariant 26: the units module's own comment reworded" pass \
	"$units_gate" \
	"$(py 'edit("crates/api/src/money/units.rs", "/// A unit at or above this share of its cap reads", "/// A unit at or above this fraction of its cap reads")')"

# invariant 28: Felyx names only the models a run is known to have used.
# Product mutants; the gate is the copilot tests whose names carry "model".
model_gate='cargo test -q --no-fail-fast -p genaryx-copilot model'

run_case "invariant 28: list_runs hands the last model over as the run's model again" fail \
	"$model_gate" \
	"$(py 'edit("crates/copilot/src/tools/cloud.rs", "if let Some(m) = map.remove(\"model\") {", "if let Some(m) = map.get(\"model\").cloned() {")')" \
	"list_runs_names_the_model_as_the_last_model_and_passes_the_models_through"

run_case "invariant 28: the answer check ignores what the tools said about models" fail \
	"$model_gate" \
	"$(py 'edit("crates/copilot/src/grounding.rs", "let fact = facts.run_models.get(*run)?;", "let fact = facts.run_models.get(*run).filter(|_| false)?;")')" \
	"a_single_model_claim_for_a_run_whose_models_are_unknown_is_flagged"

run_case "invariant 28: the phrase list's own comment reworded" pass \
	"$model_gate" \
	"$(py 'edit("crates/copilot/src/grounding.rs", "/// Phrases that make a model claim about a run", "/// Phrases that make a model claim about one run")')"

# invariant 33: Stop agent bans the agent at vouchryx, kills its open runs and
# takes the ceremony, and a step that fails is never a stop. Product mutants;
# the gate is the tests that hold each, against the stub vouchryx and Cloud.
stop_gate='cargo test -q -p genaryx-api --test it agent_stop_test'

run_case "invariant 33: the stop asks for a revocation and no ban" fail \
	"$stop_gate" \
	"$(py 'edit("crates/api/src/delegation/stop.rs", "        \"ban\": true,\n", "        \"ban\": false,\n")')" \
	"the_stop_asks_vouchryx_for_a_ban_not_a_revocation"

run_case "invariant 33: a revocation with no ban counted as a ban" fail \
	"$stop_gate" \
	"$(py 'edit("crates/api/src/delegation/stop.rs", "let banned = answer.get(\"banned\").and_then(Value::as_bool) == Some(true);", "let banned = true;")')" \
	"a_revocation_without_a_confirmed_ban_is_never_a_stop"

run_case "invariant 33: the agent's runs are not killed" fail \
	"$stop_gate" \
	"$(py 'edit("crates/api/src/delegation/stop.rs", "for run in open.iter().take(MAX_RUNS_PER_STOP) {", "for run in open.iter().take(0) {")')" \
	"stopping_an_agent_bans_it_and_kills_each_open_run"

run_case "invariant 33: agent_stop taken out of the ceremony column" fail \
	'cargo test -q -p genaryx-web --bin genaryx-web agent_stop' \
	"$(py 'edit("crates/web/src/roles.rs", "    gate(\"agent_stop\", Role::Admin, true),", "    gate(\"agent_stop\", Role::Admin, false),")')" \
	"agent_stop_with_a_passkey_and_no_assertion_is_428"

run_case "invariant 33: the stop module's own comment reworded" pass \
	"$stop_gate" \
	"$(py 'edit("crates/api/src/delegation/stop.rs", "/// The most runs one stop kills.", "/// The largest number of runs one stop kills.")')"

echo
echo "=== and the one this estate learned the hard way ==="
echo "    a gate whose subject is gone must SAY so, not report OK on nothing"

# The gate now reads the exit code rather than discarding it, so a broken suite
# is reported as a broken suite instead of as a lying README.
run_case "readme-numbers: a failing suite is not a smaller count" fail \
	'./scripts/readme-numbers.sh' \
	"$(py 'edit("crates/core/src/bus.rs", "#[cfg(test)]", "#[cfg(test)]\nmod teeth_forced_failure {\n    #[test]\n    fn this_test_fails_on_purpose() { assert!(false, \"planted by gates-have-teeth\"); }\n}\n\n#[cfg(test)]")')" \
	"the suite did not pass"

run_case "one-test-binary-per-crate: no crates left to look at" fail \
	'./scripts/one-test-binary-per-crate.sh' \
	"$(py 'import shutil; shutil.rmtree("crates")')" \
	"measured nothing"

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
