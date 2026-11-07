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
		! grep -qF -- "$needle" <<<"$out"; then
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

# invariant 38: a plane's text reaches Felyx fenced, and nothing in an answer
# points at what no tool returned. The first four read the source; the rest
# are product mutants, against the tests that hold each.
fence_gate='./scripts/felyx-fences-plane-text.sh'

run_case "felyx-fences-plane-text: the event of a summary goes out bare" fail \
	"$fence_gate" \
	"$(py 'edit("crates/copilot/src/agent.rs", "Message::user(fence_text(\"event\", event))", "Message::user(event)")')" \
	"a user message this gate does not know"

run_case "felyx-fences-plane-text: a read tool's result goes through the console's constructor" fail \
	"$fence_gate" \
	"$(py 'edit("crates/copilot/src/agent.rs", "let fenced = FencedResult::plane(&call.name, &value);", "let fenced = FencedResult::console(&value);")')" \
	"FencedResult::console( appears 3 times"

run_case "felyx-fences-plane-text: a case file goes to the model through the plain constructor" fail \
	"$fence_gate" \
	"$(py 'edit("crates/copilot/src/agent.rs", "let fenced = FencedResult::case(case);", "let fenced = FencedResult::plane(\"case_file\", case);")')" \
	"a case file no longer goes through FencedResult::case"

run_case "felyx-fences-plane-text: the agent file is gone" fail \
	"$fence_gate" \
	"$(py 'import os; os.rename("crates/copilot/src/agent.rs", "crates/copilot/src/agent_moved.rs")')" \
	"measured nothing"

run_case "felyx-fences-plane-text: a comment reworded" pass \
	"$fence_gate" \
	"$(py 'edit("crates/copilot/src/agent.rs", "/// One draft, as far as the checks got with it.", "/// One draft, as far as the checks went with it.")')"

fenced_gate='cargo test -q --no-fail-fast -p genaryx-copilot untrusted'
references_gate='cargo test -q --no-fail-fast -p genaryx-copilot references'

run_case "invariant 38: a field nobody listed stays plain" fail \
	"$fenced_gate" \
	"$(py 'edit("crates/copilot/src/untrusted.rs", "Value::String(s) => Value::String(self.fence(key, s, subjects)),", "Value::String(_) => value.clone(),")')" \
	"event_text_reaches_the_model_fenced_and_cannot_close_the_fence"

run_case "invariant 38: a closing tag passes through the escape" fail \
	"$fenced_gate" \
	"$(py 'edit("crates/copilot/src/untrusted.rs", "\x27<\x27 | \x27\\u{ff1c}\x27 | \x27\\u{fe64}\x27 => out.push_str(\"&lt;\"),", "\x27\\u{ff1c}\x27 | \x27\\u{fe64}\x27 => out.push_str(\"&lt;\"),")')" \
	"hostile_text_never_closes_the_fence"

run_case "invariant 38: an id that is a sentence stays plain" fail \
	"$fenced_gate" \
	"$(py 'edit("crates/copilot/src/untrusted.rs", "if STRUCTURED_KEYS.contains(&key) && token_shaped(s) => {", "if STRUCTURED_KEYS.contains(&key) => {")')" \
	"a_run_id_that_is_a_sentence_is_fenced_and_grounds_nothing"

run_case "invariant 38: an invented id passes" fail \
	"$references_gate" \
	"$(py 'edit("crates/copilot/src/references.rs", "let known = self.grounded(&reference.token)\n", "let known = true || self.grounded(&reference.token)\n")')" \
	"an_invented_run_id_is_sent_back_once"

run_case "invariant 38: an invented amount passes" fail \
	"$references_gate" \
	"$(py 'edit("crates/copilot/src/references.rs", "if self.supported(&amount) {", "if true || self.supported(&amount) {")')" \
	"a_tool_amount_supports_itself_and_its_rounding_and_nothing_else"

run_case "invariant 38: an explanation keeps its chain with an invented link" fail \
	"$references_gate" \
	"$(py 'edit("crates/copilot/src/agent.rs", "if self.no_incident.is_some() || (brief.incident.is_some() && !self.references.is_empty()) {", "if self.no_incident.is_some() {")')" \
	"an_explanation_citing_an_invented_run_gives_no_chain_at_all"

run_case "invariant 38: a card goes out whatever the text asked" fail \
	"$references_gate" \
	"$(py 'edit("crates/copilot/src/references.rs", "if let Some(span) = self.demanding_span(kind, &target) {\n                return Err", "if let Some(span) = self.demanding_span(kind, &target).filter(|_| false) {\n                return Err")')" \
	"text_asking_for_the_action_refuses_it_even_on_a_real_target"

run_case "invariant 38: a card on a run no tool returned goes out" fail \
	"$references_gate" \
	"$(py 'edit("crates/copilot/src/references.rs", "if !whole_fleet && !self.grounded_as(target_kind(kind), &target) {", "if false && !whole_fleet && !self.grounded_as(target_kind(kind), &target) {")')" \
	"a_proposal_on_a_run_no_tool_returned_is_refused"

run_case "invariant 38: the check's own comment reworded" pass \
	"$references_gate" \
	"$(py 'edit("crates/copilot/src/references.rs", "/// One statement the draft cannot support.", "/// A statement the draft cannot support.")')"

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

# invariant 36: the console tails the streams GENARYX_EVENTS_EXTRA names beside
# its events directory, read only, so the Keyryx witness's findings reach it.
# Product mutants; the gate is the bus feed's tests whose names carry "extra".
extra_gate='cargo test -q --no-fail-fast -p genaryx-api --lib extra'

run_case "invariant 36: the extra streams are named and never tailed" fail \
	"$extra_gate" \
	"$(py 'edit("crates/api/src/bus/feed.rs", "    for extra in &paths.extra {\n        if !is_regular_file(extra)", "    for extra in paths.extra.iter().take(0) {\n        if !is_regular_file(extra)")')" \
	"a_keyryx_finding_in_an_extra_stream_outside_the_events_dir_reaches_the_store"

run_case "invariant 36: a relative entry is taken instead of named" fail \
	"$extra_gate" \
	"$(py 'edit("crates/api/src/bus/feed.rs", "        if !path.is_absolute() {", "        if false {")')" \
	"the_extra_streams_setting_keeps_absolute_paths_and_names_every_entry_it_skips"

run_case "invariant 36: a stream named twice is tailed twice" fail \
	"$extra_gate" \
	"$(py 'edit("crates/api/src/bus/feed.rs", "        if !out.contains(&path) {\n            out.push(path);\n        }\n    }\n    out\n}\n\n/// True only", "        if true {\n            out.push(path);\n        }\n    }\n    out\n}\n\n/// True only")')" \
	"a_stream_named_both_in_the_events_dir_and_as_an_extra_is_tailed_once"

run_case "invariant 36: a line claiming another plane in an extra stream is stored as that plane" fail \
	"$extra_gate" \
	"$(py 'edit("crates/core/src/ingest.rs", ".filter(|want| event.source != **want)", ".filter(|_| false)")')" \
	"a_line_claiming_another_source_in_an_extra_stream_is_quarantined_not_stored"

run_case "invariant 36: an extra stream is listed through a symlink" fail \
	"$extra_gate" \
	"$(py 'edit("crates/api/src/bus/feed.rs", "    std::fs::symlink_metadata(path)\n", "    std::fs::metadata(path)\n")')" \
	"an_extra_stream_that_is_a_symlink_or_a_directory_is_not_read"

run_case "invariant 36: the sources function's own comment reworded" pass \
	"$extra_gate" \
	"$(py 'edit("crates/api/src/bus/feed.rs", "/// Every stream the bus should tail right now:", "/// Every stream the bus ought to tail right now:")')"

# invariant 37: the case file for an incident is assembled with no model, every
# question has a line, every fact names its plane, its row and a digest, and an
# identity refusal is filed under its key. Product mutants; the gates are the
# case module's own tests and the integration test against a real store.
case_gate='cargo test -q --no-fail-fast -p genaryx-api --lib case::'
case_it_gate='cargo test -q --no-fail-fast -p genaryx-api --test it case_file_test'

run_case "invariant 37: a plane that did not answer is left out of the case" fail \
	"$case_gate" \
	"$(py 'edit("crates/api/src/case/mod.rs", "        lines.push(line);\n    }\n\n    let mut case = CaseFile {", "        if line.state != LineState::Unavailable {\n            lines.push(line);\n        }\n    }\n\n    let mut case = CaseFile {")')" \
	"a_plane_that_did_not_answer_is_a_line_in_the_case"

run_case "invariant 37: an identity refusal is filed under the agent it claimed" fail \
	"$case_gate" \
	"$(py 'edit("crates/api/src/case/mod.rs", "    let subject = filed_under(&event.type_, &event.agent_id, event.data.as_ref());\n    let is_key", "    let subject = event.agent_id.clone();\n    let is_key")')" \
	"an_identity_refusal_case_is_filed_under_its_key"

run_case "invariant 37: a digest is taken of something other than the row" fail \
	"$case_gate" \
	"$(py 'edit("crates/api/src/case/mod.rs", "Sha256::digest(bytes)", "Sha256::digest(b\"\")")')" \
	"a_digest_is_the_sha256_of_the_row_and_changes_with_it"

run_case "invariant 37: a plane that failed to answer reads as one that had nothing" fail \
	"$case_it_gate" \
	"$(py 'edit("crates/api/src/case/commands.rs", "        Err(e) => Answer::Unavailable { reason: why(&e) },", "        Err(_) => nothing(),")')" \
	"a_case_with_no_reachable_plane_names_each_one"

run_case "invariant 37: the run timeline is left in insertion order" fail \
	"$case_it_gate" \
	"$(py 'edit("crates/api/src/case/commands.rs", "events.sort_by_key(|e| (parse_ts_ms(&e.ts).unwrap_or(i64::MIN), e.id));", "events.sort_by_key(|e| e.id);")')" \
	"the_case_for_a_bus_incident_reads_its_event_the_run_timeline_and_similar_past_incidents"

run_case "invariant 37: the similar incidents include the incident itself" fail \
	'cargo test -q --no-fail-fast -p genaryx-core --test it similar_events' \
	"$(py 'edit("crates/core/src/store.rs", "FROM events WHERE type = ?1 AND id < ?2 ORDER BY id DESC LIMIT ?3", "FROM events WHERE type = ?1 AND id <= ?2 ORDER BY id DESC LIMIT ?3")')" \
	"similar_events_are_the_earlier_events_of_the_same_type_newest_first"

run_case "invariant 37: reading a case becomes an admin act" fail \
	'cargo test -q --no-fail-fast -p genaryx-web --bin genaryx-web reading_a_case' \
	"$(py 'edit("crates/web/src/roles.rs", "gate(\"case_file\", Role::Viewer, false)", "gate(\"case_file\", Role::Admin, false)")')" \
	"reading_a_case_is_a_viewer_read"

run_case "invariant 37: idryx is asked about the agent an identity refusal claimed" fail \
	"$case_it_gate" \
	"$(py 'edit("crates/api/src/case/mod.rs", "    let agent = (!is_key).then(|| subject.clone());", "    let agent = Some(event.agent_id.clone());")')" \
	"an_identity_refusal_asks_idryx_nothing_at_all"

run_case "invariant 37: the alerts of other identities are listed as the subject's" fail \
	"$case_it_gate" \
	"$(py 'edit("crates/api/src/case/commands.rs", "                .filter(|a| a.identity == agent)", "                .filter(|_| true)")')" \
	"the_subject_agents_identity_and_blast_radius_come_from_idryx"

run_case "invariant 37: the approvals of other runs are listed as this run's decisions" fail \
	"$case_it_gate" \
	"$(py 'edit("crates/api/src/case/commands.rs", "                .filter(|a| a.run_id == run_id)", "                .filter(|_| true)")')" \
	"wardryx_gives_the_policy_status_and_the_approvals_of_the_run"

run_case "invariant 37: the error-text bound's own comment reworded" pass \
	"$case_gate" \
	"$(py 'edit("crates/api/src/case/mod.rs", "/// The longest piece of a plane'"'"'s own error text a case keeps.", "/// The largest piece of a plane'"'"'s own error text a case keeps.")')"

# invariant 39: Felyx reads one case file instead of running a tool loop. Product
# mutants; the gate is the scripted-model tests whose file name carries the case.
case_read_gate='cargo test -q --no-fail-fast -p genaryx-copilot --test it felyx_reads_the_case_test'

run_case "invariant 39: the model is offered the tools while it reads a case" fail \
	"$case_read_gate" \
	"$(py 'edit("crates/copilot/src/agent.rs", "        let tools = if case.is_some() {\n            Vec::new()\n        } else {\n            self.registry.specs()\n        };", "        let tools = self.registry.specs();")')" \
	"felyx_reads_the_case_it_is_given_and_asks_for_no_tool"

run_case "invariant 39: a tool the model asks for is run while it reads a case" fail \
	"$case_read_gate" \
	"$(py 'edit("crates/copilot/src/agent.rs", "                if case.is_some() {\n                    let refusal =", "                if false {\n                    let refusal =")')" \
	"a_model_that_asks_for_a_tool_while_reading_a_case_gets_none"

run_case "invariant 39: a plane's row text reaches the model plain" fail \
	"$case_read_gate" \
	"$(py 'edit("crates/copilot/src/untrusted.rs", "\"row\" => self.value(v, \"row\", &subjects),", "\"row\" => v.clone(),")')" \
	"plane_text_in_a_case_row_reaches_the_model_fenced_and_grounds_nothing"

run_case "invariant 39: the incident a case is about is not checked against the case" fail \
	"$case_read_gate" \
	"$(py 'edit("crates/copilot/src/agent.rs", "            incident: Some(&incident),\n        };\n        let read = CaseRead", "            incident: None,\n        };\n        let read = CaseRead")')" \
	"a_case_with_no_incident_id_gives_no_chain"

run_case "invariant 39: the case prompt's own doc comment reworded" pass \
	"$case_read_gate" \
	"$(py 'edit("crates/copilot/src/agent.rs", "/// The system prompt for reading one case file.", "/// The system prompt used for reading one case file.")')"

# invariant 40: a candidate policy is proved by `wardryx replay`, in both halves,
# and a proof that could not be made is never a proof of nothing. Product
# mutants; the gates are the proof module's own tests and the integration test
# with a stand-in wardryx.
proof_gate='cargo test -q --no-fail-fast -p genaryx-api --lib proof::'
proof_it_gate='cargo test -q --no-fail-fast -p genaryx-api --test it policy_proof_test'

run_case "invariant 40: a change on another run is counted as the incident's" fail \
	"$proof_gate" \
	"$(py 'edit("crates/api/src/proof/mod.rs", ".partition(|row| row.run.text == incident_run);", ".partition(|_| true);")')" \
	"the_incident_is_stopped_only_by_turning_its_own_allow_into_a_deny_or_a_hold"

run_case "invariant 40: a loosening of the incident's own decision counts as stopping it" fail \
	"$proof_gate" \
	"$(py 'edit("crates/api/src/proof/mod.rs", ".any(|r| r.from == \"allow\" && (r.to == \"deny\" || r.to == \"hold\"));", ".any(|r| r.from != r.to);")')" \
	"the_incident_is_stopped_only_by_turning_its_own_allow_into_a_deny_or_a_hold"

run_case "invariant 40: a diverged replay is trusted" fail \
	"$proof_gate" \
	"$(py 'edit("crates/api/src/proof/mod.rs", "let trustworthy = report.outcomes.diverged == 0;", "let trustworthy = true;")')" \
	"a_diverged_replay_proves_nothing"

run_case "invariant 40: a report with a forged row is believed" fail \
	"$proof_gate" \
	"$(py 'edit("crates/api/src/proof/mod.rs", "if rows.len() as u64 != changed {", "if false {")')" \
	"a_report_that_contradicts_itself_is_refused"

run_case "invariant 40: outcomes that do not sum to the decisions replayed are believed" fail \
	"$proof_gate" \
	"$(py 'edit("crates/api/src/proof/mod.rs", "if outcomes.total() != replayed {", "if false {")')" \
	"a_report_that_contradicts_itself_is_refused"

run_case "invariant 40: the child inherits the console's environment" fail \
	"$proof_it_gate" \
	"$(py 'edit("crates/api/src/proof/commands.rs", "        .env_clear()\n", "")')" \
	"the_child_gets_no_environment_but_a_path"

run_case "invariant 40: the candidate is left behind when the run ends" fail \
	"$proof_it_gate" \
	"$(py 'edit("crates/api/src/proof/commands.rs", "let _ = std::fs::remove_dir_all(&self.0);", "let _ = &self.0;")')" \
	"the_proof_runs_wardryx_replay_with_exactly_its_arguments_and_removes_the_candidate"

run_case "invariant 40: the replay's deadline is not enforced" fail \
	"$proof_it_gate" \
	"$(py 'edit("crates/api/src/proof/commands.rs", "Ok(None) if Instant::now() >= deadline => {", "Ok(None) if false => {")')" \
	"a_replay_that_hangs_is_killed_at_its_deadline"

run_case "invariant 40: an empty candidate is run" fail \
	"$proof_it_gate" \
	"$(py 'edit("crates/api/src/proof/commands.rs", "if candidate.trim().is_empty() {", "if false {")')" \
	"a_candidate_that_cannot_be_given_is_refused_before_anything_runs"

run_case "invariant 40: the limit on proofs in flight is ignored" fail \
	"$proof_gate" \
	"$(py 'edit("crates/api/src/proof/slots.rs", "(n < limit).then_some(n + 1)", "Some(n + 1)")')" \
	"slots_hand_out_exactly_their_limit"

run_case "invariant 40: a slot is never given back" fail \
	"$proof_gate" \
	"$(py 'edit("crates/api/src/proof/slots.rs", "self.in_use.fetch_sub(1, Ordering::AcqRel);", "let _ = &self.in_use;")')" \
	"a_released_slot_is_taken_again"

run_case "invariant 40: a busy slot runs its proof anyway" fail \
	"$proof_it_gate" \
	"$(py 'edit("crates/api/src/proof/commands.rs", "let _slot = slots\n        .try_acquire()\n        .map_err(|busy| ProofError::Busy { limit: busy.limit })?;", "let _slot = slots.try_acquire().ok();")')" \
	"a_busy_slot_is_an_answer_not_a_queue"

run_case "invariant 40: a replay that examined nothing answers no for the incident" fail \
	"$proof_gate" \
	"$(py 'edit("crates/api/src/proof/mod.rs", "stops_this_incident: (trustworthy && candidate.replayable > 0).then_some(stops),", "stops_this_incident: trustworthy.then_some(stops),")')" \
	"a_replay_in_which_nothing_could_be_replayed_says_nothing_about_the_incident"
# invariant 41: a candidate budget is proved by `tokenfuse backtest`, in both
# halves, and a proof that could not be made is never a proof of nothing. Product
# mutants; the gates are the budget module's own tests and the integration test
# with a stand-in tokenfuse.
proof_budget_it_gate='cargo test -q --no-fail-fast -p genaryx-api --test it budget_proof_test'

run_case "invariant 41: the amount handed to the CLI is the plain text of the micros" fail \
	"$proof_gate" \
	"$(py 'edit("crates/api/src/proof/budget.rs", "(micros as f64 + 0.5) / 1e6", "micros as f64 / 1e6")')" \
	"an_amount_is_handed_to_the_cli_as_text_it_reads_back_exactly"

run_case "invariant 41: a zero limit is accepted as a candidate" fail \
	"$proof_gate" \
	"$(py 'edit("crates/api/src/proof/budget.rs", "if micros == 0 {", "if false {")')" \
	"amounts_are_whole_micro_dollars_or_refused"

run_case "invariant 41: a report for another candidate is believed" fail \
	"$proof_gate" \
	"$(py 'edit("crates/api/src/proof/budget.rs", "if echoed != asked.describe() {", "if false {")')" \
	"a_backtest_report_for_another_candidate_is_refused"

run_case "invariant 41: a projection that is not the spend less the saving is believed" fail \
	"$proof_gate" \
	"$(py 'edit("crates/api/src/proof/budget.rs", "if Some(projected) != spent.checked_sub(saved) {", "if false {")')" \
	"a_backtest_report_that_contradicts_itself_is_refused"

run_case "invariant 41: calls blocked in no run are believed" fail \
	"$proof_gate" \
	"$(py 'edit("crates/api/src/proof/budget.rs", "if (runs_affected > 0) != (calls_blocked > 0) {", "if false {")')" \
	"a_backtest_report_that_contradicts_itself_is_refused"

run_case "invariant 41: the whole trace's blocks count as stopping the incident" fail \
	"$proof_gate" \
	"$(py 'edit("crates/api/src/proof/budget.rs", "let stops = run.calls_blocked > 0;", "let stops = trace.calls_blocked > 0;")')" \
	"the_incident_is_stopped_only_when_its_own_run_has_a_call_blocked"

run_case "invariant 41: the rest of the trace is the whole trace, the incident included" fail \
	"$proof_gate" \
	"$(py 'edit("crates/api/src/proof/budget.rs", "runs_affected: trace.runs_affected - run.runs_affected,", "runs_affected: trace.runs_affected,")')" \
	"the_rest_of_the_trace_is_the_whole_trace_less_the_incident"

run_case "invariant 41: a split over calls at the same step is called exact" fail \
	"$proof_gate" \
	"$(py 'edit("crates/api/src/proof/budget.rs", "elsewhere_exact: !tied,", "elsewhere_exact: true,")')" \
	"a_run_with_calls_at_the_same_step_marks_the_split_approximate"

run_case "invariant 41: a copy that disagrees with its replay is believed" fail \
	"$proof_gate" \
	"$(py 'edit("crates/api/src/proof/budget.rs", "if run.calls_total != copied {", "if false {")')" \
	"reports_that_cannot_describe_the_same_trace_are_refused"

run_case "invariant 41: a whole trace with fewer calls than the run is believed" fail \
	"$proof_gate" \
	"$(py 'edit("crates/api/src/proof/budget.rs", "if trace.calls_total < run.calls_total {", "if false {")')" \
	"reports_that_cannot_describe_the_same_trace_are_refused"

run_case "invariant 41: a run id that could break the query is put in it" fail \
	"$proof_gate" \
	"$(py 'edit("crates/api/src/proof/budget.rs", "if !run.bytes().all(is_name_byte) {", "if false {")')" \
	"a_run_id_is_put_in_a_query_only_when_it_is_a_plain_name"

run_case "invariant 41: an empty copy is taken for a proof of nothing" fail \
	"$proof_budget_it_gate" \
	"$(py 'edit("crates/api/src/proof/budget_commands.rs", "if count == 0 {", "if false {")')" \
	"a_run_the_trace_does_not_hold_is_not_a_proof_of_nothing"

run_case "invariant 41: each child has a deadline of its own" fail \
	"$proof_budget_it_gate" \
	"$(py 'edit("crates/api/src/proof/budget_commands.rs", "    started + timeout\n", "    Instant::now() + timeout\n")')" \
	"all_the_children_of_one_proof_share_one_deadline"

run_case "invariant 41: a busy slot runs a budget proof anyway" fail \
	"$proof_budget_it_gate" \
	"$(py 'edit("crates/api/src/proof/budget_commands.rs", "let _slot = slots\n        .try_acquire()\n        .map_err(|busy| ProofError::Busy { limit: busy.limit })?;", "let _slot = slots.try_acquire().ok();")')" \
	"a_budget_proof_meets_a_busy_slot_as_an_answer_not_a_queue"

run_case "invariant 41: the copy of the run is left behind when the proof ends" fail \
	"$proof_budget_it_gate" \
	"$(py 'edit("crates/api/src/proof/commands.rs", "let _ = std::fs::remove_dir_all(&self.0);", "let _ = &self.0;")')" \
	"the_proof_runs_four_children_in_order_over_the_right_directories_and_removes_the_copy"

run_case "invariant 41: the children inherit the console's environment" fail \
	"$proof_budget_it_gate" \
	"$(py 'edit("crates/api/src/proof/commands.rs", "        .env_clear()\n", "")')" \
	"the_proof_runs_four_children_in_order_over_the_right_directories_and_removes_the_copy"

# invariant 42: a case is sealed as one canonical document whose digest is the target
# of a chain line. Product mutants; the gates are the seal module's own tests and
# the integration test over a real store and a real journal.
seal_gate='cargo test -q --no-fail-fast -p genaryx-api --lib seal::'
seal_it_gate='cargo test -q --no-fail-fast -p genaryx-api --test it case_seal_test'

run_case "invariant 42: members are sorted by their UTF-8 bytes" fail \
	"$seal_gate" \
	"$(py 'edit("crates/api/src/seal/jcs.rs", "keys.sort_by(|a, b| a.encode_utf16().cmp(b.encode_utf16()));", "keys.sort_by(|a, b| a.as_bytes().cmp(b.as_bytes()));")')" \
	"members_are_sorted_by_utf16_code_units_not_by_bytes"

run_case "invariant 42: an integer a double cannot hold is written rounded" fail \
	"$seal_gate" \
	"$(py 'edit("crates/api/src/seal/jcs.rs", "return if i.unsigned_abs() <= MAX_EXACT_INT {", "return if true {")')" \
	"an_integer_a_double_cannot_hold_is_refused_and_one_it_can_is_written_whole"

run_case "invariant 42: a number past 1e21 is written in positional form" fail \
	"$seal_gate" \
	"$(py 'edit("crates/api/src/seal/jcs.rs", "} else if 0 < n && n <= 21 {", "} else if 0 < n && n <= 22 {")')" \
	"numbers_are_written_the_way_ecmascript_writes_them"

run_case "invariant 42: a control character is left unescaped" fail \
	"$seal_gate" \
	"$(py 'edit("crates/api/src/seal/jcs.rs", "c if (c as u32) < 0x20 =>", "c if (c as u32) < 0x1f =>")')" \
	"strings_are_escaped_the_way_json_stringify_escapes_them"

run_case "invariant 42: a pack that is not in canonical form is accepted" fail \
	"$seal_gate" \
	"$(py 'edit("crates/api/src/seal/mod.rs", "if canonical(&value).as_deref() != Ok(text) {", "if false {")')" \
	"a_pack_is_the_pack_its_digest_names_only_if_it_is_whole_canonical_and_of_this_format"

run_case "invariant 42: a pack that hashes to another digest is accepted" fail \
	"$seal_gate" \
	"$(py 'edit("crates/api/src/seal/mod.rs", "if found != digest {", "if false {")')" \
	"a_pack_is_the_pack_its_digest_names_only_if_it_is_whole_canonical_and_of_this_format"

run_case "invariant 42: a digest of any length is turned into a file name" fail \
	"$seal_gate" \
	"$(py 'edit("crates/api/src/seal/mod.rs", "hex.len() == 64", "hex.len() >= 1")')" \
	"only_a_sha256_in_lowercase_hex_is_a_digest"

run_case "invariant 42: a note of any length is sealed" fail \
	"$seal_gate" \
	"$(py 'edit("crates/api/src/seal/mod.rs", "if note.chars().count() > NOTE_MAX_CHARS {", "if false {")')" \
	"a_decision_is_trimmed_bounded_and_free_of_control_characters"

run_case "invariant 42: a seal that was not journaled is stored anyway" fail \
	"$seal_it_gate" \
	"$(py 'edit("crates/api/src/seal/commands.rs", "if !journaled {", "if false {")')" \
	"a_seal_that_cannot_be_journaled_stores_nothing"

run_case "invariant 42: a digest that is not a digest is looked up on disk" fail \
	"$seal_it_gate" \
	"$(py 'edit("crates/api/src/seal/commands.rs", "if !is_digest(digest) {", "if false {")')" \
	"a_digest_that_is_not_a_digest_never_reaches_the_filesystem"

run_case "invariant 42: the canonical error's own doc comment reworded" pass \
	"$seal_gate" \
	"$(py 'edit("crates/api/src/seal/jcs.rs", "/// Why a value has no canonical form.", "/// Why a value has no canonical form at all.")')"
# invariant 43: the bench scores a named cause, the ids an answer cites and the
# remedy that follows from the cause against what is known to be true. Product
# mutants; the gate is the bench module's own tests.
bench_gate='cargo test -q --no-fail-fast -p genaryx-api --lib bench::'

run_case "invariant 43: two causes named in one answer count as the first" fail \
	"$bench_gate" \
	"$(py 'edit("crates/api/src/bench/mod.rs", "[one] => Named::Cause(*one),", "[one, ..] => Named::Cause(*one),")')" \
	"a_cause_is_named_when_one_cause_is_named_in_a_sentence_and_no_other"

run_case "invariant 43: one word of a cause names it" fail \
	"$bench_gate" \
	"$(py 'edit("crates/api/src/bench/mod.rs", "group.iter().all(|word| sentence.contains(word))", "group.iter().any(|word| sentence.contains(word))")')" \
	"a_cause_is_named_when_one_cause_is_named_in_a_sentence_and_no_other"

run_case "invariant 43: a withheld chain still counts as citing ids that exist" fail \
	"$bench_gate" \
	"$(py 'edit("crates/api/src/bench/mod.rs", "unsupported.is_empty() && !text.starts_with(WITHHELD_PREFIX)", "unsupported.is_empty()")')" \
	"the_first_two_questions_are_judged_on_the_cause_named_and_on_what_was_left_unsupported"

run_case "invariant 43: any cause named counts as the true one" fail \
	"$bench_gate" \
	"$(py 'edit("crates/api/src/bench/mod.rs", "named == Named::Cause(truth)", "matches!(named, Named::Cause(_))")')" \
	"the_first_two_questions_are_judged_on_the_cause_named_and_on_what_was_left_unsupported"

run_case "invariant 43: a remedy that does not apply is counted as one that was replayed" fail \
	"$bench_gate" \
	"$(py 'edit("crates/api/src/bench/mod.rs", "remedy_applicable: scores.iter().filter(|s| s.remedy_stops.is_some()).count(),", "remedy_applicable: scores.len(),")')" \
	"a_remedy_that_does_not_apply_is_counted_as_neither_a_pass_nor_a_miss"

run_case "invariant 43: a packet never gives its cause away" fail \
	"$bench_gate" \
	"$(py 'edit("crates/api/src/bench/mod.rs", ".find(|w| lower.contains(w))", ".find(|w| w.is_empty())")')" \
	"a_packet_gives_the_cause_away_only_by_its_words"

run_case "invariant 43: a refused key calls for a policy" fail \
	"$bench_gate" \
	"$(py 'edit("crates/api/src/bench/mod.rs", "Cause::UnknownKey => None,", "Cause::UnknownKey => Some(RemedyFor::Policy),")')" \
	"the_remedy_a_cause_calls_for_is_the_one_that_follows_from_it"

run_case "invariant 43: the bench's own doc comment reworded" pass \
	"$bench_gate" \
	"$(py 'edit("crates/api/src/bench/mod.rs", "/// The causes the registry knows.", "/// The causes that the registry knows.")')"
# invariant 44: a remedy is typed, checked, proved where a proof exists, and routed
# to a person; nothing here applies it. Product mutants; the gates are the remedy
# module's own tests and the integration test over a real store.
remedy_gate='cargo test -q --no-fail-fast -p genaryx-api --lib remedy::'
remedy_it_gate='cargo test -q --no-fail-fast -p genaryx-api --test it remedy_check_test'

run_case "invariant 44: an event type is matched by its prefix" fail \
	"$remedy_gate" \
	"$(py 'edit("crates/api/src/remedy/mod.rs", "if e.event_type == event_type {", "if event_type.starts_with(e.event_type) {")')" \
	"an_event_type_is_classified_by_the_table_and_any_other_is_unclassified"

run_case "invariant 44: a type nobody listed is guessed to be a policy incident" fail \
	"$remedy_gate" \
	"$(py 'edit("crates/api/src/remedy/mod.rs", "    (IncidentClass::Unclassified, None)\n", "    (IncidentClass::Policy, None)\n")')" \
	"an_event_type_is_classified_by_the_table_and_any_other_is_unclassified"

run_case "invariant 44: a refused key is given a scenario that rehearses another failure" fail \
	"$remedy_gate" \
	"$(py 'edit("crates/api/src/remedy/mod.rs", "entry(\"identity_mismatch\", IncidentClass::Identity, None),", "entry(\"identity_mismatch\", IncidentClass::Identity, Some(RUNAWAY)),")')" \
	"an_event_type_is_classified_by_the_table_and_any_other_is_unclassified"

run_case "invariant 44: an identity incident is offered a stop of the agent it claimed" fail \
	"$remedy_gate" \
	"$(py 'edit("crates/api/src/remedy/mod.rs", "IncidentClass::Identity | IncidentClass::Unclassified => &[],", "IncidentClass::Identity | IncidentClass::Unclassified => &[RemedyKind::StopAgent],")')" \
	"an_identity_incident_is_offered_no_remedy_and_says_why"

run_case "invariant 44: a stop is routed to the command that lifts a ban" fail \
	"$remedy_gate" \
	"$(py 'edit("crates/api/src/remedy/mod.rs", "command: \"agent_stop\",", "command: \"agent_lift\",")')" \
	"a_stop_or_a_revocation_routes_to_its_ceremony_command_with_its_subject"

run_case "invariant 44: a stop of any text is accepted as a stop of an agent" fail \
	"$remedy_gate" \
	"$(py 'edit("crates/api/src/remedy/mod.rs", "if is_agent_id(subject) {", "if true {")')" \
	"a_remedy_is_checked_before_anything_is_proved_or_routed"

run_case "invariant 44: every remedy is said to fit every incident" fail \
	"$remedy_it_gate" \
	"$(py 'edit("crates/api/src/remedy/commands.rs", "let fits_class = offered(class).contains(&request.remedy.kind());", "let fits_class = true;")')" \
	"an_identity_refusal_is_offered_nothing_and_a_stop_of_its_claimed_agent_does_not_fit"

run_case "invariant 44: a scenario is said to be present without looking" fail \
	"$remedy_it_gate" \
	"$(py 'edit("crates/api/src/remedy/commands.rs", "in_scenario_dir: dir.map(|d| d.join(scenario).is_file()),", "in_scenario_dir: Some(true),")')" \
	"a_spend_incident_is_classified_offered_a_budget_and_a_drill"

run_case "invariant 44: the kinds' own doc comment reworded" pass \
	"$remedy_gate" \
	"$(py 'edit("crates/api/src/remedy/mod.rs", "/// The kinds of remedy there are.", "/// The kinds of remedy that there are.")')"

run_case "invariant 41: the candidate's own doc comment reworded" pass \
	"$proof_gate" \
	"$(py 'edit("crates/api/src/proof/budget.rs", "/// A candidate budget, in the units the trace keeps.", "/// A candidate budget, in the units that the trace keeps.")')"

run_case "invariant 40: the outcomes' own doc comment reworded" pass \
	"$proof_gate" \
	"$(py 'edit("crates/api/src/proof/mod.rs", "/// The seven rows `wardryx replay` counts.", "/// The seven rows that `wardryx replay` counts.")')"

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

# Invariant 45. Under pipefail a pipe into grep -q fails when the writer is still writing, so a healthy tree
# goes red at random on a loaded machine. The first cases plant the construct; the middle ones are what
# must not be flagged; the last ones break the gate itself, because a reader that has stopped matching
# reports what a clean tree reports.

run_case "no-reader-leaves-a-pipe-early: a gate pipes its text into grep -q again" fail \
	'./scripts/no-reader-leaves-a-pipe-early.sh' \
	"$(py 'edit("scripts/felyx-fences-plane-text.sh", "if ! grep -q \x27FencedResult::plane(&call.name, &value)\x27 <<<\"$(product \"$agent\")\"; then", "if ! product \"$agent\" | grep -q \x27FencedResult::plane(&call.name, &value)\x27; then")')" \
	'scripts/felyx-fences-plane-text.sh:'

run_case "no-reader-leaves-a-pipe-early: the pipe is carried over a line" fail \
	'./scripts/no-reader-leaves-a-pipe-early.sh' \
	"$(py 'edit("scripts/felyx-fences-plane-text.sh", "#!/usr/bin/env bash\n", "#!/usr/bin/env bash\nprintf \x27%s\\n\x27 \"$x\" \\\n  | grep -q y\n")')" \
	'scripts/felyx-fences-plane-text.sh:'

run_case "no-reader-leaves-a-pipe-early: a workflow step reads through the pipe" fail \
	'./scripts/no-reader-leaves-a-pipe-early.sh' \
	"$(py 'edit(".github/workflows/ci.yml", "run: ./scripts/readme-numbers.sh", "run: printf x | grep -q x")')" \
	'.github/workflows/ci.yml:'

run_case "no-reader-leaves-a-pipe-early: the words are only in a comment" pass \
	'./scripts/no-reader-leaves-a-pipe-early.sh' \
	"$(py 'edit("scripts/felyx-fences-plane-text.sh", "#!/usr/bin/env bash\n", "#!/usr/bin/env bash\n# printf x | grep -q x\n")')"

run_case "no-reader-leaves-a-pipe-early: the words are a command handed to another shell" pass \
	'./scripts/no-reader-leaves-a-pipe-early.sh' \
	"$(py 'edit("scripts/felyx-fences-plane-text.sh", "#!/usr/bin/env bash\n", "#!/usr/bin/env bash\nsh -c \"printf x | grep -q x\"\n")')"

run_case "no-reader-leaves-a-pipe-early: the words are in a here-document" pass \
	'./scripts/no-reader-leaves-a-pipe-early.sh' \
	"$(py 'edit("scripts/felyx-fences-plane-text.sh", "#!/usr/bin/env bash\n", "#!/usr/bin/env bash\ncat <<\x27EOF\x27\nprintf x | grep -q x\nEOF\n")')"

run_case "no-reader-leaves-a-pipe-early: grep -c reads to the end of a pipe" pass \
	'./scripts/no-reader-leaves-a-pipe-early.sh' \
	"$(py 'edit("scripts/felyx-fences-plane-text.sh", "#!/usr/bin/env bash\n", "#!/usr/bin/env bash\nprintf x | grep -c x >/dev/null\n")')"

run_case "no-reader-leaves-a-pipe-early: a here-string has no pipe to leave" pass \
	'./scripts/no-reader-leaves-a-pipe-early.sh' \
	"$(py 'edit("scripts/felyx-fences-plane-text.sh", "#!/usr/bin/env bash\n", "#!/usr/bin/env bash\ngrep -q x <<<x\n")')"

run_case "no-reader-leaves-a-pipe-early: a substitution in quotes reads through the pipe" fail \
	'./scripts/no-reader-leaves-a-pipe-early.sh' \
	"$(py 'edit("scripts/felyx-fences-plane-text.sh", "#!/usr/bin/env bash\n", "#!/usr/bin/env bash\nx=\"$(printf y | grep -m1 y)\"\n")')" \
	'scripts/felyx-fences-plane-text.sh:2 pipes into `grep -m1`'

run_case "no-reader-leaves-a-pipe-early: a long option leaves as early as a short one" fail \
	'./scripts/no-reader-leaves-a-pipe-early.sh' \
	"$(py 'edit("scripts/felyx-fences-plane-text.sh", "#!/usr/bin/env bash\n", "#!/usr/bin/env bash\nprintf y | grep --quiet y\n")')" \
	'scripts/felyx-fences-plane-text.sh:2 pipes into `grep --quiet`'

run_case "no-reader-leaves-a-pipe-early: a quote left open loses the reader its place" fail \
	'./scripts/no-reader-leaves-a-pipe-early.sh' \
	"$(py 's = open("scripts/felyx-fences-plane-text.sh").read()
open("scripts/felyx-fences-plane-text.sh", "w").write(s + "echo \"open\n")')" \
	'lost its place'

run_case "no-reader-leaves-a-pipe-early: the reader stops knowing -q" fail \
	'./scripts/no-reader-leaves-a-pipe-early.sh' \
	"$(py 'edit("scripts/no-reader-leaves-a-pipe-early.sh", "if ch in \"qmlL\":", "if ch in \"mlL\":")')" \
	'the reader missed'

run_case "no-reader-leaves-a-pipe-early: the reader stops reporting what it finds" fail \
	'./scripts/no-reader-leaves-a-pipe-early.sh' \
	"$(py 'edit("scripts/no-reader-leaves-a-pipe-early.sh", "found.append((m.start(), flag))", "pass")')" \
	'the reader missed'

run_case "no-reader-leaves-a-pipe-early: the reader starts reading quoted text" fail \
	'./scripts/no-reader-leaves-a-pipe-early.sh' \
	"$(py 'edit("scripts/no-reader-leaves-a-pipe-early.sh", "blank(i, end + 1)", "pass")')" \
	'the reader flagged'

run_case "no-reader-leaves-a-pipe-early: the control writes less than a pipe holds" fail \
	'./scripts/no-reader-leaves-a-pipe-early.sh' \
	"$(py 'edit("scripts/no-reader-leaves-a-pipe-early.sh", "WRITER = \"seq 1 400000\"", "WRITER = \"seq 1 3\"")')" \
	'it must fail every time'

run_case "no-reader-leaves-a-pipe-early: no pipe is read" fail \
	'./scripts/no-reader-leaves-a-pipe-early.sh' \
	"$(py 'edit("scripts/no-reader-leaves-a-pipe-early.sh", "pipes += len(PIPE.findall(masked))", "pipes += 0")')" \
	'measured NOTHING'

run_case "no-reader-leaves-a-pipe-early: no shell file is read" fail \
	'./scripts/no-reader-leaves-a-pipe-early.sh' \
	"$(py 'edit("scripts/no-reader-leaves-a-pipe-early.sh", "if path.endswith(\".sh\") or path.startswith(\".githooks/\") or (first.startswith(\"#!\") and \"bash\" in first):", "if False:")')" \
	'measured NOTHING'

run_case "no-reader-leaves-a-pipe-early: no workflow step is read" fail \
	'./scripts/no-reader-leaves-a-pipe-early.sh' \
	"$(py 'edit("scripts/no-reader-leaves-a-pipe-early.sh", "elif path.startswith(\".github/workflows/\") and", "elif path.startswith(\".github/flows/\") and")')" \
	'measured NOTHING'

# invariant 46: the raw approval token stays in its answer and in component
# memory. The gate's own cases first, then product mutants through cargo.
token_gate='./scripts/approval-token-stays-in-memory.sh'

run_case "approval-token: a module outside the list reads the wire field" fail \
	"$token_gate" \
	"$(py 'edit("apps/web/src/lib/recentEvents.ts", "export ", "export const _teeth = (o: { approval_token: string }) => o.approval_token;\nexport ")')" \
	'recentEvents.ts names approval_token'

run_case "approval-token: a new module holds a granted token" fail \
	"$token_gate" \
	"$(py 'edit("apps/web/src/lib/recentEvents.ts", "export ", "export type _Teeth = import(\"./approvalToken\").GrantedToken;\nexport ")')" \
	'recentEvents.ts holds a granted token'

run_case "approval-token: a holder writes to browser storage" fail \
	"$token_gate" \
	"$(py 'edit("apps/web/src/components/ApprovalsInbox.tsx", "      setCopy(\"copied\");", "      setCopy(\"copied\");\n      localStorage.setItem(\"t\", raw);")')" \
	'reaches past this component'

run_case "approval-token: a holder logs to the console" fail \
	"$token_gate" \
	"$(py 'edit("apps/web/src/components/PolicyView.tsx", "        const granted = grantedTokenFrom(id, outcome);", "        const granted = grantedTokenFrom(id, outcome);\n        console.log(granted);")')" \
	'reaches past this component'

run_case "approval-token: the policy view stays mounted across navigation" fail \
	"$token_gate" \
	"$(py 'edit("apps/web/src/components/AppShell.tsx", "{view === \"policy\" && (\n          <PolicyView", "{(\n          <PolicyView")')" \
	'does not mount PolicyView only while it is the active view'

run_case "approval-token: the copilot crate names the call that mints it" fail \
	"$token_gate" \
	"$(py 'edit("crates/copilot/src/tools/wardryx.rs", "pub(super) struct ApprovalsInbox;", "// decide_approval\npub(super) struct ApprovalsInbox;")')" \
	'the copilot crate names the token'

run_case "approval-token: the raw token derives Debug" fail \
	"$token_gate" \
	"$(py 'edit("crates/api/src/policy/commands.rs", "#[derive(Clone, Serialize)]\n#[serde(transparent)]", "#[derive(Clone, Debug, Serialize)]\n#[serde(transparent)]")')" \
	'RawApprovalToken derives Debug'

run_case "approval-token: the raw token gains a way to be read" fail \
	"$token_gate" \
	"$(py 'edit("crates/api/src/policy/commands.rs", "/// The token to hand the operator for this answer", "impl RawApprovalToken {\n    pub fn text(&self) -> &str { &self.0 }\n}\n\n/// The token to hand the operator for this answer")')" \
	'RawApprovalToken has a way to be read'

run_case "approval-token: the wardryx response prints its token" fail \
	"$token_gate" \
	"$(py 'edit("crates/connectors/src/wardryx.rs", "#[derive(Clone, Deserialize)]\npub struct ApprovalDecideResponse", "#[derive(Clone, Debug, Deserialize)]\npub struct ApprovalDecideResponse")')" \
	'ApprovalDecideResponse derives Debug'

run_case "approval-token: a third reader of the field in the policy commands" fail \
	"$token_gate" \
	"$(py 'edit("crates/api/src/policy/commands.rs", "    let (bus_recorded, bus_error) = journal(&client, &rec);", "    let _third = result.as_ref().ok().map(|r| r.approval_token.clone());\n    let (bus_recorded, bus_error) = journal(&client, &rec);")')" \
	'reads .approval_token 3 times'

run_case "approval-token: the answer to a decision may be cached" fail \
	"$token_gate" \
	"$(py 'edit("crates/web/src/dispatch.rs", "Ok(no_store(reply(", "Ok(std::convert::identity(reply(")')" \
	'is not marked no-store'

run_case "approval-token: the module that holds it is gone" fail \
	"$token_gate" \
	"$(py 'import os
os.remove("apps/web/src/lib/approvalToken.ts")')" \
	'measured nothing'

run_case "approval-token: no module holds a token any more" fail \
	"$token_gate" \
	"$(py 'import subprocess
for f in subprocess.check_output(["git", "ls-files", "apps/web/src"], text=True).split():
    if f.endswith((".ts", ".tsx")) and not f.endswith((".test.ts", ".test.tsx")):
        s = open(f).read()
        t = s.replace("GrantedTokenPanel", "Panel").replace("grantedTokenFrom", "fromGrant").replace("GrantedToken", "Grant")
        if t != s:
            open(f, "w").write(t)')" \
	'measured nothing'

run_case "approval-token: a test may name the field, and a read of the host is no leak" pass \
	"$token_gate" \
	"$(py 'edit("apps/web/src/lib/recentEvents.test.ts", "import ", "// approval_token in a test file is a fixture\nimport ")
edit("apps/web/src/components/PolicyView.tsx", "        void refresh();\n      } catch (err) {", "        void refresh();\n        void window.location.host;\n      } catch (err) {")')"

token_tests='cargo test -q -p genaryx-api --test it approval_token_test'

run_case "invariant 46: a grant answers with no raw token" fail \
	"$token_tests" \
	"$(py 'edit("crates/api/src/policy/commands.rs", "            approval_token: raw_token_of(decision, &resp),", "            approval_token: None,")')" \
	"a_grant_answers_with_the_raw_token_wardryx_minted"

run_case "invariant 46: the journal keeps a copy of the token" fail \
	"$token_tests" \
	"$(py 'edit("crates/api/src/policy/commands.rs", "        verify_result: verify_result.clone(),\n    };\n    let (bus_recorded, bus_error) = journal(&client, &rec);", "        verify_result: format!(\"{verify_result} {:?}\", result.as_ref().ok().and_then(|r| r.approval_token.clone())),\n    };\n    let (bus_recorded, bus_error) = journal(&client, &rec);")')" \
	"the_raw_token_reaches_no_journal_row_no_bus_line_and_no_file"

run_case "invariant 46: the raw token prints" fail \
	"$token_tests" \
	"$(py 'edit("crates/api/src/policy/commands.rs", "        f.write_str(\"RawApprovalToken(<redacted>)\")", "        f.write_str(&self.0)")')" \
	"the_answer_prints_without_the_raw_token"

run_case "invariant 46: a Keyryx token is read from its first segment" fail \
	"$token_tests" \
	"$(py 'edit("crates/connectors/src/wardryx.rs", "[payload, _] | [_, payload, _] => *payload,", "[payload, _] | [payload, _, _] => *payload,")')" \
	"a_grant_carries_the_claims_it_shows_in_both_token_formats"

run_case "invariant 46: a grant with no decodable claims loses its token" fail \
	"$token_tests" \
	"$(py 'edit("crates/api/src/policy/commands.rs", "            approval_token: raw_token_of(decision, &resp),", "            approval_token: token.as_ref().and_then(|_| raw_token_of(decision, &resp)),")
edit("crates/api/src/policy/commands.rs", "            token,\n            approval_token", "            token: token.clone(),\n            approval_token")')" \
	"a_grant_whose_claims_cannot_be_read_still_answers_with_the_raw_token"

run_case "invariant 46: a decode error repeats what it refused" fail \
	'cargo test -q -p genaryx-connectors --lib a_decode_error_never_repeats_what_it_refused' \
	"$(py 'edit("crates/connectors/src/wardryx.rs", ".map_err(|_| WardryxError::BadToken(\"invalid claims JSON\".to_string()))", ".map_err(|e| WardryxError::BadToken(format!(\"invalid claims JSON: {e}\")))")')" \
	"a_decode_error_never_repeats_what_it_refused"

run_case "invariant 46: the decide path logs the token" fail \
	'cargo test -q -p genaryx-web --bin genaryx-web a_granted_token_reaches_no_log_line' \
	"$(py 'edit("crates/api/src/policy/commands.rs", "    let (http_status, verify_result, token, summary) =\n        describe_decision_result(decision, &id, &result);", "    eprintln!(\"decided: {:?}\", result.as_ref().ok().and_then(|r| r.approval_token.clone()));\n    let (http_status, verify_result, token, summary) =\n        describe_decision_result(decision, &id, &result);")')" \
	"stderr holds"

run_case "invariant 46: the answer leaves no-store off" fail \
	'cargo test -q -p genaryx-web --bin genaryx-web a_grant_through_the_router_answers_the_raw_token_and_keeps_no_copy' \
	"$(py 'edit("crates/web/src/dispatch.rs", "Ok(no_store(reply(", "Ok(std::convert::identity(reply(")')" \
	"a_grant_through_the_router_answers_the_raw_token_and_keeps_no_copy"

run_case "invariant 46: the token type's own comment reworded" pass \
	"$token_tests" \
	"$(py 'edit("crates/api/src/policy/commands.rs", "/// The token to hand the operator for this answer: a grant", "/// The token to give the operator for this answer: a grant")')"

# A standing grant (CLAUDE.md invariant 47): the command, on a stub wardryx, and the gates it
# sits behind. Each name is a test that has to go red.
grant_tests='cargo test -q -p genaryx-api --test it standing_grant_test'
grant_gates='cargo test -q -p genaryx-web --bin genaryx-web standing_grant'

run_case "invariant 47: policy_grant_tool leaves the ceremony column" fail \
	"$grant_gates" \
	"$(py 'edit("crates/web/src/roles.rs", "gate(\"policy_grant_tool\", Role::Admin, true)", "gate(\"policy_grant_tool\", Role::Admin, false)")')" \
	"a_standing_grant_with_a_passkey_and_no_assertion_is_428"

run_case "invariant 47: an approver may write a standing grant" fail \
	"$grant_gates" \
	"$(py 'edit("crates/web/src/roles.rs", "gate(\"policy_grant_tool\", Role::Admin, true)", "gate(\"policy_grant_tool\", Role::Approver, true)")')" \
	"an_approver_is_refused_by_the_role_gate_on_a_standing_grant"

run_case "invariant 47: the grant stops asking for a proven chain" fail \
	"$grant_tests" \
	"$(py 'edit("crates/api/src/policy/standing_grant.rs", "        deny_if_chain_unproven: true,\n        ..Policy::default()", "        deny_if_chain_unproven: false,\n        ..Policy::default()")')" \
	"a_grant_writes_the_policy_agent_allow_writes"

run_case "invariant 47: a hold that came with a static key loses it" fail \
	"$grant_tests" \
	"$(py 'edit("crates/api/src/policy/standing_grant.rs", "        _ if plan.static_key => &STATIC_SOURCES,\n", "")')" \
	"a_hold_that_came_with_a_static_key_is_not_turned_into_a_refusal"

run_case "invariant 47: a grant overwrites what the agent holds" fail \
	"$grant_tests" \
	"$(py 'edit("crates/api/src/policy/standing_grant.rs", "let mut tools: Vec<String> = kept_tools.to_vec();", "let mut tools: Vec<String> = Vec::new();")')" \
	"a_grant_adds_to_the_policy_the_agent_already_holds"

run_case "invariant 47: an id with no trust domain is written" fail \
	"$grant_tests" \
	"$(py 'edit("crates/api/src/policy/standing_grant.rs", "    if !pins_trust_domain(&held.agent_id) {\n", "    if held.agent_id.is_empty() {\n")')" \
	"an_agent_with_no_trust_domain_reaches_no_policy_and_is_journaled"

run_case "invariant 47: a hold of another kind is written" fail \
	"$grant_tests" \
	"$(py 'edit("crates/api/src/policy/standing_grant.rs", "    if held.hold_kind() != Some(NO_GRANT) {\n", "    if held.hold_kind() == Some(\"never\") {\n")')" \
	"a_hold_of_another_kind_reaches_no_policy"

run_case "invariant 47: a refused attempt leaves no record" fail \
	"$grant_tests" \
	"$(py 'edit("crates/api/src/policy/commands.rs", "            let _ = journal_grant(\n                &client,\n                &operator,\n                &id,\n                params,\n                \"refused\",\n                0,\n                &verify_result,\n            );\n", "            let _ = (&params, &verify_result);\n")')" \
	"an_agent_with_no_trust_domain_reaches_no_policy_and_is_journaled"

run_case "invariant 47: a comment in the policy's own file reworded" pass \
	"$grant_tests" \
	"$(py 'edit("crates/api/src/policy/standing_grant.rs", "//! A standing grant: \"always allow this tool for this agent\", asked on a hold.", "//! A standing grant: \"always allow this tool for this agent\", offered on a hold.")')"

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
