# CLAUDE.md, working instructions for genaryx

These instructions apply to any model working in this repo. Read this file
before writing code. It holds process and invariants only: **no status.**
Status goes stale, and a stale instruction file is worse than none.

## Read before you change anything

1. `README.md`, for what the console is and what it is not.
2. `docs/`, and the D-decisions in the private `TAIPANBOX/itrat-console` repo.
   Those are the architecture; this repo is the implementation of it.
3. The crate boundaries: `core`, `api`, `connectors`, `copilot`, `signing`,
   `web`. They are a layering, not folders.

## What this is

The browser control room over the agent-governance stack. Money, policy,
identity, quality, crypto, memory, drills and signed evidence in one window, on
the operator's own infrastructure. Public, Apache-2.0, and **nothing about it is
sold**.

**It is web-only.** The native desktop shells were deleted on 2026-07-24 and the
phone and watch branch is cancelled. Do not reintroduce either, and do not
mention the cancelled branch in code, docs, or copy.

## Building the published demo

The demo on it-rat.com is `npm run build:demo` in `apps/web`, never
`npm run build`. Both flags in it are load-bearing and neither is guessable:

- `--mode mock` loads `.env.mock`, which sets `VITE_GENARYX_MOCK=1`. Only that
  build wraps `Console` in `demo/DemoFunnel.tsx`, the sign-in mimic and the
  connect step. `--mode web` is the real product, so a static copy of it has no
  box to reach and renders "No answer from the box".
- `--base=./` makes the asset paths relative. The default writes `/assets/...`,
  which resolves against the SITE root rather than `/demo/`, so nothing loads
  and the page is blank.

Both were rediscovered on 2026-08-03 by building without them and watching each
break in turn, because the command lived only in whoever ran it last. Copy
`dist/` into the site's `demo/`, and delete the previous hashed asset files:
the names change per build and stale ones are served forever otherwise.

## Gates

```sh
./scripts/no-cloud-credentials.sh
./scripts/no-fabricated-rows.sh
./scripts/web-only-and-unpriced.sh
./scripts/features-are-bound.sh      # invariant 10
./scripts/readme-numbers.sh          # runs the whole suite; slow
./scripts/gates-have-teeth.sh        # invariant 7; needs a clean tree
```

This list did not exist until 2026-08-09. The four gates above were named only
inside the invariants that own them, and CI ran all four, so there was no one
place a person could read to know what to run.

`readme-numbers.sh` takes minutes because it runs `cargo test --workspace`.

## Hard invariants

Each one carries how it is held today. Use `(gate: ...)`, `(test: ...)`,
`(partly gated: ...)` or `(not enforced)`, and use the weakest one that is
true. An invariant with no check, written as though it had one, is worse than
an absent invariant.

1. **The console never stores cloud credentials.** Multi-cloud inventory is
   read-only and runs through the operator's own CLI, already authenticated on
   their machine. A console that holds cloud keys is a target, and it changes
   what this product is.
   *(gate: `scripts/no-cloud-credentials.sh`, which checks two structural
   things: no cloud credential environment variable is read anywhere, and no
   provider SDK is declared. An SDK exists to authenticate, so pulling one in is
   the same decision arriving under another name.)*
2. **A sensitive command requires a per-action ceremony.** Kill, budget
   change, approval, the two operator WireGuard commands (issue a peer,
   revoke a peer), and revoking a delegation, each need a fresh passkey
   confirmation. Six, not three: `crates/web/src/main.rs`'s
   `SENSITIVE_COMMANDS` is the list, and this file said three until
   2026-08-05, five until 2026-09-24. Not a session, not a role check alone:
   the ceremony is per action, because the whole point is that a stolen
   session cannot pull the switch.
   *(partly gated: router-level tests in `crates/web/src/main.rs` drive the
   real axum router through the whole ceremony, and hold that an enrolled
   caller is refused without an assertion, that the command and argument
   bindings are enforced, that enrolling and removing a passkey each need a
   factor the session does not carry, and that with
   `GENARYX_WEB_REQUIRE_PASSKEY=1` all six are refused when nobody is
   enrolled. What is NOT held is the default configuration: with the setting
   off and no passkey enrolled, a sensitive command still runs and is
   journaled software-signed. So the invariant holds on a box that enrolled a
   passkey or set the variable, and is a documented fallback otherwise.)*
3. **Every privileged action is journaled into a verified hash chain.** An
   action that happened without a chain entry is indistinguishable from one that
   did not happen. And it must be the CONSOLE's chain: a console_command
   appended into a product's file breaks that product's chain from its next
   event onward, because every producer on this bus seeds its chain from the
   file tail once at open and advances it in memory.
   *(partly gated: `crates/core/tests/console_chain_test.rs` holds the chain
   itself, that the console's lines stay one chain with another writer
   appending to the same file throughout and with several commands landing at
   once, and `crates/core/src/command.rs`'s own tests hold that a line and its
   newline are one write and that a failed write does not advance the chain.
   `crates/api/src/money/state.rs` holds that the file is the console's own
   and none of the six product files. What is NOT held is the "every" in the
   sentence: nothing structural proves the NEXT privileged action added will
   journal at all. The lifecycle blocks were the one that did not, for months,
   while the note explaining why said the signing path was unreachable and it
   was reachable two functions away.)*
4. **The console shows the operator's real records, never a mock.** A card with
   no data says it has no data. Inventing a plausible number to fill a panel is
   the single worst thing this product can do, because the entire proposition is
   that what you see is what happened.
   *(partly gated: `scripts/no-fabricated-rows.sh`, structurally and in the two
   places this actually erodes: which modules may import fixture ROWS at all,
   and that no `catch` block anywhere reaches for them. Verified by running it
   against the real pre-fix `recentEvents.ts`, which it fails.
   `apps/web/src/lib/recentEvents.test.ts` holds the behaviour: a backend that
   throws yields no rows and an `error` source, never fixtures. What is NOT
   held is the rest of the sentence, every card in every panel saying it has
   no data rather than showing a placeholder; the gate covers the fixture
   stream, not every individual empty state.)*
5. **Web-only, and no cancelled surface returns.** This is settled.
   *(gate: `scripts/web-only-and-unpriced.sh`, by artefact rather than by
   vocabulary: a shell leaves a config, a project file, sources or a manifest
   entry behind, and those are unambiguous. Honest history in the PAST tense,
   recording that shells existed and were removed, is deliberately untouched.)*
6. **Nothing here is paid.** No purchase surface, no gated feature. This was
   removed once already, estate-wide.
   *(gate: `scripts/web-only-and-unpriced.sh`, in what the console SHOWS.
   "upgrade" has an honest meaning here, software-signed actions upgrade to
   hardware-confirmed and an agent is literally named `dependency-upgrader`, so
   a word list would cry wolf and get disabled. What is forbidden is a
   purchase-surface component and `upgrade_url` reaching a component.)*

7. **A check must be able to tell "did not fail" from "did not run", and every
   gate here has been made to fail on purpose to prove it can.**
   `readme-numbers.sh` says in its own words that a suite reporting no tests
   means it measured nothing. That sentence was true and nothing had re-run it.

   It also carried a nearer relative of the same fault, and this is the part
   worth keeping. It took the test count from a run whose exit code it
   discarded and whose stderr it sent to /dev/null. Cargo stops after the first
   crate that fails, so ONE failing test cut the workspace from six crates to
   two, the sum fell from 688 to 479, and the gate reported that the README was
   lying. The README was correct throughout. **A number read from a broken run
   is not a smaller number, it is a different measurement wearing the same
   units.** It now passes `--no-fail-fast`, reads the exit code, and refuses to
   compare anything when the suite did not pass.

   The failing test was itself the same shape one level down: two live-skip
   tests picked their traces directory with `is_dir()` while their own comments
   asked for "populated", and the installer creates that directory empty. Two
   copies of one block, both fixed.

   The three grep-shaped gates are the other risk here: a pattern that stops
   matching reports success, and each prints OK from an empty result set.
   *(gate: `scripts/gates-have-teeth.sh`, 6 cases: four real faults, one
   non-fault, and one planted test failure that must be reported as a broken
   suite rather than as a stale badge. The non-fault is the one worth keeping:
   `apps/web/src/lib/recentEvents.ts` is the single module allowed to import
   fixtures, and a gate that flagged it would be flagging the design it
   protects.)*

   **What it does not cover.** It cannot test itself. It proves each gate
   catches the faults named in it, not every fault of that kind.

8. **A number this console prints is about the question, or it says which part
   of the question it could not reach.** A count answers for its whole window,
   or the response carries a field naming the columns that fall short. Never a
   figure that is accurate about itself and false about what was asked.

   This is invariant 4's sibling and the harder half of it. Invariant 4 is about
   inventing rows; this is about a real number under a wrong label, which no
   check that looks at the number can catch, because the number is correct.

   It was found on 2026-08-11 by measuring, not by reading. `stats_counts` read
   the N most recent events and tallied them, with N a cap chosen when the store
   was scratch and held a few thousand lines. Durable history made that cap a
   truncation nobody could see: @measured `crates/api/tests/stats_scale.rs` at
   42 agents and 100 events a day, ninety days is 378,000 events and the
   frontend asked for 20,000, so "how often was this agent stopped in the last
   thirty days" was answered from about five per cent of the window, under a
   sentence reading "counted from 20,000 event(s) in the last 30 day(s)". Every
   word of that was true. An operator had no way to tell it from an estate where
   20,000 things happened.

   The counts now come from a SQL aggregate that reads no rows and cannot be
   capped. What remains capped is the narrow second read of events whose own
   `data` must be opened, and `StatsPanel::detail_truncated` says when it was
   hit, with the affected columns marked in the header rather than only excused
   in a note.
   *(test: `crates/api/src/stats/mod.rs`,
   `a_small_detail_cap_does_not_shrink_the_counts` drives the real fold through
   a real store with the cap set far below the data and holds that the counts
   are the whole window, and `a_capped_detail_read_says_the_descriptive_columns_are_partial`
   holds the other half, that a capped detail read is declared rather than
   presented as complete. Both were run against the pre-fix code first and both
   failed there. Two further tests hold the split itself against drift:
   `every_type_that_needs_its_data_is_read_in_full` fails if an attribution rule
   is added without its event type being fetched, and
   `every_amount_field_pair_is_actually_read` fails if the query and the reader
   disagree about the budget field names.

   The second case is the QUARANTINE, and it is the same sentence one level
   further out. A line that fails the envelope is kept, with its file, offset,
   raw bytes and the validator's reason, on the principle that a malformed line
   must never silently vanish. Nothing read it back until 2026-08-11:
   `quarantine_count` had no caller and the only report anywhere was one
   `eprintln!` at startup, to stderr, once. So the line did not vanish and the
   operator could not tell the difference, because the console does not go
   blank when it happens. It shows the rest of the bus, correctly, and the
   broken producer's agents just look idle. `aws-comparable-176` is the real
   instance: twelve events refused for an `agent_id` with no `agent://` prefix,
   and every count about that agent was exactly right and described nothing
   that happened.
   `crates/api/tests/quarantine_is_visible_test.rs` drives the REAL captured
   campaign through the REAL ingest path and holds that the console reports the
   refusal, names the validator's own reason, and points at the file and
   offset; that a clean bus claims the check rather than rendering blank; and
   that a box with no store refuses to report a clean bus.

   What is NOT held is the sentence's scope: these seven cover the Statistics
   panel and the quarantine, and nothing structural stops the next capped or
   unread thing elsewhere in the console from doing the same.)*

9. **The console accepts every envelope version the contract obliges it to,
   and refuses a claimed subject by decision rather than by accident.**
   agent-passport SPEC 6.4.1 (1.0, 2026-09-12): a consumer MUST accept event
   v0.1, v0.2 and v1.0, and a consumer that does not model a claimed subject
   refuses, and counts, a line whose `agent_id` carries `claimed:` (SPEC 3.3).
   This console has no model for a claim, so `Conformer` refuses such a line
   under one fixed reason, `CLAIMED_SUBJECT_REFUSED`, and the quarantine panel
   shows it as one row with a count. v0.3 stays refused by version, which SPEC
   6.4 allows.

   Two choices worth keeping. The refusal runs after schema validation, so the
   reason names the decision and not a regex: only v1.0's pattern admits the
   form at all. And the reason is one string without the subject in it, because
   the panel groups by reason: a producer writing claimed subjects is one row
   with a total, not one row per line and no total.
   *(test: `crates/core/tests/conform_test.rs`,
   `a_v1_0_event_is_accepted_and_resolved`,
   `a_v1_0_claimed_subject_is_refused_under_one_reason`,
   `v0_3_stays_refused_by_version`,
   `the_vendored_v1_0_schema_widens_only_the_subject`;
   `crates/core/tests/ingest_test.rs`,
   `claimed_subjects_are_quarantined_under_one_reason_and_counted`. The two
   claimed-subject tests were run first against the conformer with the refusal
   removed and both failed there: the claimed line validated and reached the
   store.)*

10. **The console's durable bus store lands where the console can write, and
    a store that lands anywhere but the first choice says so.** `console.sqlite`
    lived under `<TAIPAN_HOME>/genaryx/<env>` and nowhere else. Both launchers
    point `TAIPAN_HOME` at `/etc/genaryx/taipan`, a root-owned directory
    holding one read-only mount of `environments/`, so `create_dir_all` failed
    with a bare `Permission denied` and the Bus Explorer was empty on every
    stack-single install and every stack-k8s cluster, at v0.1.2 and v1.0.0
    alike; measured 2026-09-14 on both rigs and reproduced on this image with
    a clean events directory (genaryx#71). The error named no path, so it was
    first read as a problem with the bus files, which it never was.

    Three candidates, first creatable wins: `<GENARYX_STATE_DIR>/<env>` when
    the operator names a state directory, `<TAIPAN_HOME>/genaryx/<env>` (the
    desktop path, unchanged), then `<HOME>/.taipan/genaryx/<env>`, which in
    both launchers is the console's own state volume. A skipped candidate is
    named on stderr with its reason, so a store one candidate down is never a
    silent move; a store nowhere creatable names every path tried and the
    variable that fixes it. `GENARYX_STATE_DIR` is declared in
    `components.json` and held by `crates/web/tests/manifest.rs`.
    *(tests: `the_store_lands_under_home_when_taipan_home_cannot_be_written`,
    `an_uncreatable_store_names_every_path_it_tried`,
    `an_explicit_state_dir_wins_over_a_writable_taipan_home`,
    `the_store_dir_is_asked_for_in_the_order_state_dir_taipan_home_then_home`
    in `crates/api/src/bus/feed.rs`; red first as a compile failure on the
    unfixed tree; two mutants caught, the HOME fallback removed and the search
    stopping at the first refusal. Scenarios:
    `features/the-bus-store-lands-where-the-console-can-write.feature`, four,
    each bound; gate: `scripts/features-are-bound.sh`, three cases in
    `gates-have-teeth.sh`. Not covered: a `TAIPAN_HOME` that is writable but
    on a volume the launcher later drops, which is a launcher question.)*

11. **A key the IdP removed stays trusted until a refresh SUCCEEDS, and the
    console's IdP sign-in no longer depends on a static JWKS alone.**
    `GENARYX_WEB_OIDC_JWKS_URL` (`crates/web/src/oidc.rs`) is an optional
    `https://` alternative to the static `GENARYX_WEB_OIDC_JWKS`: the console
    fetches and caches the JWKS itself, so an IdP's routine key rotation
    (Okta: about four times a year, unannounced; Entra: no fixed interval,
    immediate in an emergency) no longer locks every operator out of IdP
    sign-in until someone edits configuration by hand. The static path stays
    the default and the air-gap path. Setting both JWKS variables, or a URL
    that is not `https://`, refuses to start (exit non-zero, one message
    naming the problem) rather than guessing which the operator meant.

    A fetch happens: once at startup (best-effort - a failed one does not
    stop the console, which starts with an empty live set and the local
    Argon2id account still reachable as break-glass); before a verification
    when the current set is older than one hour (`MAX_AGE`); on an unknown
    `kid`. EITHER reason (age or kid) is bounded by the SAME cooldown
    (`COOLDOWN`, five minutes): a fetch happens only when due AND the last
    attempt (the startup one counts) is at least `COOLDOWN` old, or there has
    been no attempt yet. A `tokio::sync::Mutex` held across the whole
    decide-then-fetch step makes concurrent sign-ins single-flight: never two
    fetches for one gap. The real fetcher (`ReqwestFetcher`) times out at
    5 s, never follows a redirect, and reads the body incrementally, capped
    at 1 MiB.

    A fetched body is refused outright, and the last good set stays in
    force, unless it is valid JSON with a non-empty `keys` array and no key
    carries a private-key member (`d`, `p`, `q`, `dp`, `dq`, `qi`, `k`) - this
    last check runs on the RAW JSON, before `jsonwebtoken`'s own
    `RSAKeyParameters`/`EllipticCurveKeyParameters` silently drop any field
    they do not model (there is no `deny_unknown_fields` on either), which is
    the only point a leaked private key is still visible; by the time a
    typed `JwkSet` exists, a `d` the source JSON carried would already be
    gone and unrecoverable. A key of type `oct` also refuses the whole set (a
    published symmetric secret is the same compromise as a leaked private
    key). Any OTHER key type this code cannot verify with - OKP (Ed25519)
    among them, legitimately published by some IdPs beside RSA/EC - is
    DROPPED from the usable set instead (one debug line naming the kid and
    kty), never a reason to refuse the whole fetch: refusing would lock every
    operator at such an IdP out of sign-in over a type gap, not a
    compromise. Only a set left with nothing usable after dropping is
    refused, as empty.

    Marked `@claude 2026-09-24`: a decision taken under delegated authority,
    open to reversal. Corrected the same day, same authority: the age-due
    refresh path originally bypassed the cooldown entirely (so a failed
    startup fetch, or an outage past the hour, retried on every single
    verification instead of at most once per cooldown), and any key type
    other than RSA/EC originally refused the whole fetched set (so one OKP
    key beside an IdP's RSA/EC ones locked out every operator there). Both
    corrected below; the first draft's shape is kept only in the mutants that
    now prove the fix.
    *(test: `crates/web/src/oidc.rs`'s
    `a_rotated_key_verifies_after_the_kid_miss_refresh`,
    `an_outage_keeps_verifying_on_the_last_good_set`,
    `a_second_unknown_kid_inside_the_cooldown_makes_no_fetch`,
    `a_hundred_concurrent_unknown_kids_within_one_cooldown_make_exactly_one_fetch`,
    `an_outage_past_max_age_with_a_hundred_verifications_in_one_cooldown_makes_exactly_one_fetch`,
    `a_failed_startup_fetch_is_not_retried_faster_than_the_cooldown`,
    `after_max_age_a_refresh_that_drops_a_key_fails_that_keys_token`,
    `hostile_jwks_bodies_are_refused_and_the_last_good_set_stays` (plus
    eleven direct `validate_rejects_*`/`validate_drops_*`/`validate_accepts_*`
    cases and a 200-seed `two_hundred_seeds_of_hostile_jwks_bodies_never_panic`
    sweep), `a_mixed_rsa_and_okp_set_verifies_the_rsa_token_and_drops_the_okp_key`,
    `both_jwks_sources_set_refuses_to_start`,
    `a_plain_http_jwks_url_refuses_to_start`,
    `the_real_fetcher_refuses_a_redirect`,
    `the_real_fetcher_refuses_an_oversized_body`,
    `a_body_over_the_cap_through_the_real_fetcher_keeps_the_last_good_set`.
    Eight mutants planted in product code, each run red against its catching
    test and restored byte for byte: the cooldown ignored (refresh on every
    unknown kid), caught by
    `a_second_unknown_kid_inside_the_cooldown_makes_no_fetch`; the cooldown
    bypassed on the age-due path, caught by
    `an_outage_past_max_age_with_a_hundred_verifications_in_one_cooldown_makes_exactly_one_fetch`
    and `a_failed_startup_fetch_is_not_retried_faster_than_the_cooldown`; the
    last good set dropped on a failed fetch, caught by
    `an_outage_keeps_verifying_on_the_last_good_set`; a plain `http://` URL
    allowed, caught by `a_plain_http_jwks_url_refuses_to_start`; the MAX_AGE
    refresh skipped, caught by
    `after_max_age_a_refresh_that_drops_a_key_fails_that_keys_token`; the
    private-member check dropped, caught by
    `validate_rejects_a_leaked_private_rsa_exponent` and
    `validate_rejects_every_forbidden_member_individually` directly, and by
    `hostile_jwks_bodies_are_refused_and_the_last_good_set_stays` as a side
    effect (the leaked key wrongly replaces the trusted set); single flight
    removed (two fetches for two concurrent misses), caught by
    `a_hundred_concurrent_unknown_kids_within_one_cooldown_make_exactly_one_fetch`;
    OKP (and any other non-RSA/EC type) refusing the whole set again instead
    of being dropped, caught by
    `a_mixed_rsa_and_okp_set_verifies_the_rsa_token_and_drops_the_okp_key`,
    `validate_drops_an_okp_key_and_keeps_the_rsa_key`,
    `validate_drops_any_unrecognised_key_type_the_same_way` and
    `validate_refuses_a_set_that_is_only_an_okp_key_as_empty`.
    Scenarios: `features/oidc-live-jwks.feature`, eight, each bound; gate:
    `scripts/features-are-bound.sh`.

    Where it says nothing: no `Cache-Control` response header is honoured,
    the hour is fixed in code rather than read from the response; no mTLS or
    certificate pinning to the IdP beyond the platform's own root store; the
    static path still needs a restart to rotate its key (unchanged); the
    once-per-failure-streak warning suppression is not itself asserted
    against captured log output, only the fetch counts and the verification
    outcomes are; the 5 s timeout is not exercised by a test (a test that
    honestly proved it would cost a real 5 s per run for one line of client
    configuration); an extremely deeply nested JSON body could in principle
    exhaust `serde_json`'s recursive-descent parser before the 1 MiB cap is
    even reached, which is a known `serde_json` limitation this change does
    not specifically defend against; a dropped key's kid/kty is logged at
    `debug` only, so an operator who wants to know an IdP is publishing a key
    type this console cannot use must raise the log level to see it.)*

12. **Cutting an agent's or a user's delegated authority is the same class of
    act as a kill: admin-only, ceremony-gated, one journal entry per attempt.**
    `@claude 2026-09-24`, a decision taken under delegated authority, open to
    reversal. `delegation_revoke` (`crates/api/src/delegation`) posts to
    vouchryx's `POST /v1/revoke` with exactly one of `subject`
    (`agent://`/`user://`) or `jti`, a bounded non-empty `reason`, and the
    actor this console already uses for its own audit trail
    (`console_actor::operator_or`, the same call `journal.rs` makes).
    Configuration (`GENARYX_VOUCHRYX_URL`, `GENARYX_VOUCHRYX_REVOKE_KEY_FILE`)
    is resolved once at startup: both unset is a normal box that answers a
    named refusal and calls nobody; one set without the other, or a key file
    that cannot be read, refuses to start rather than run half-configured.
    Every attempt journals into the same command journal `money_kill_run` and
    `remote_operator_wg_revoke` write to, carrying vouchryx's REAL
    `http_status` (0 for unreachable), never a fabricated 200: success,
    refused (400/401/403), not durable (503), or unreachable each reach the
    operator and the journal as themselves.

    vouchryx's own `refuse()` (read read-only at `~/Development/vouchryx`,
    `internal/api/api.go`, origin/main `19b5211`; the checked-out `main` was
    two commits behind and lacked this) sends the identical
    `{"error":"temporarily_unavailable"}` body whether its revocation list is
    full or it accepted a revocation in memory and then failed to persist it,
    so this console cannot and does not claim which one happened, only that
    vouchryx did not confirm durability. Its 200 answer carries no `durable`
    field either (that flag exists only in vouchryx's own event stream, never
    the HTTP answer); `vouchryx_response` forwards the raw body rather than
    inventing a field.
    *(test: `crates/api/src/delegation/env.rs`'s eleven tests hold the
    configuration resolution (both unset, both set, one without the other,
    an unreadable/empty key file, a scheme-less URL, the key redacted from
    `Debug`); `crates/api/src/delegation/commands.rs`'s
    `exactly_one_of_subject_or_jti_is_required`,
    `subject_must_be_agent_or_user_scheme`,
    `reason_must_be_non_empty_and_bounded` hold argument validation, and its
    `a_200_seed_sweep_of_hostile_args_never_panics_and_never_half_validates`
    sweeps it; `crates/api/tests/delegation_revoke_test.rs` (17 tests) drives
    the real function against a hand-rolled stub vouchryx for every outcome
    (200/400/401/403/503/unreachable), the exact posted body and bearer
    header, that a misconfigured pair refuses to start, that a bad argument
    never reaches the stub (a counting stub asserts zero connections), and
    that the bearer key never appears in the journaled line or the returned
    error; `crates/web/src/main.rs`'s
    `a_viewer_is_refused_by_the_role_gate_on_delegation_revoke`,
    `an_approver_is_refused_by_the_role_gate_on_delegation_revoke`,
    `delegation_revoke_with_a_passkey_and_no_assertion_is_428` and
    `delegation_revoke_with_a_valid_assertion_runs` hold the full HTTP-level
    gate. Red first as a compile failure on the unfixed tree (the module did
    not exist); the HTTP-level gate tests were red on the unfixed tree for a
    different, real reason: `delegation_revoke` was not yet a dispatchable
    command at all, so a viewer's request 404'd rather than 403'd.

    Mutants (planted by hand, the named test went red, restored): dropped
    from `SENSITIVE_COMMANDS` caught by
    `delegation_revoke_with_a_passkey_and_no_assertion_is_428` (428 became
    400); the role moved from admin to approver caught by
    `an_approver_is_refused_by_the_role_gate_on_delegation_revoke` (403
    became 400); a vouchryx 503 mapped to the success shape caught by
    `vouchryx_503_is_reported_as_not_durable_never_as_success`; the bearer
    key appended onto the journaled `verify_result` caught by
    `the_key_never_appears_in_the_journaled_line_or_the_returned_error`,
    which printed the leaked key back in its own failure message; argument
    validation bypassed caught by
    `both_target_forms_together_are_refused_before_any_call` (and the
    counting stub recorded a real connection, since the bypass let a
    both-subject-and-jti call reach the network). Scenarios:
    `features/the-console-revokes-a-delegation.feature`, four, each bound;
    gate: `scripts/features-are-bound.sh`.

    Where it says nothing: the key file is trusted as the operator placed
    it, this console never re-derives or checks its provenance. Revoking is
    not banning in vouchryx (its own invariant 7): a token issued after the
    revocation moment is not covered, and re-issuing after a revocation is
    the expected recovery path, not a gap. The console's own UI offers only
    `subject` (the agent detail card's "Revoke delegation" button,
    `crates/web/src/lib/lifecycle.tsx`'s `RevokeDelegationButton`); revoking
    a single token by its `jti` alone still needs the command directly, not
    a console control. The mock/demo preview build has no synthetic
    vouchryx to answer against, so the button errors there like any command
    with no mock handler, which was left as is: the demo funnel's own scope
    is fixed policy (see "Building the published demo" above), and a real
    box is what this control is for.)*

## Decisions that have no gate yet

This list is debt, and it is here to stay visible rather than to be tidy.
**No invariant is now held by this file alone.** Every one of the eight carries
a gate or a test, and the ones that are partial say in their own marker which
half is held and which is not. That is the useful state, not a clean one: a
marker reading "partly gated" with the unheld half spelled out is worth more
than a green tick over a claim nobody checked.

**Invariants 3 and 4** were the last two held by prose, and both turned out to
be false about our own code when somebody finally went to check, which is the
argument for gates in one line:

- The console was appending its `console_command` lines into `tokenfuse.ndjson`
  and `qryx.ndjson`, two products' own files. Each product seeds its SPEC 6.5
  chain from the file tail once when it opens the file and advances it in
  memory, so a console line landing in between made that product's next event
  name a predecessor that was no longer the one on disk. Deterministic, not a
  race, and invisible: every line still conformed on its own. The console now
  writes `console.ndjson`.
- `recentEvents.ts` answered ANY thrown error with the `mockData.ts` fixture
  stream, so a console pointed at a box that had stopped answering showed
  fabricated agents, severities and timestamps. The mitigation was a label in
  a status bar, which is not a mitigation when the ROWS are the claim.

Two smaller things went the same way and are worth recording as the same
class. `crates/web/src/roles.rs` said "a test asserts the classified set equals
the live dispatch set, so a new command cannot be added without being placed";
the test compared two hand-maintained lists that both lived in `roles.rs`, so
a command added to `dispatch.rs` and to neither list passed. It reads
`dispatch.rs` itself now. And `scripts/no-cloud-credentials.sh` enumerated AWS,
GCP, Azure, IBM and OpenStack credential names and no Hetzner term at all,
while this console ships a Hetzner inventory connector: a
`std::env::var("HCLOUD_TOKEN")` in `crates/` passed the gate cleanly.

The pattern in all four: the claim was written when it was true of the
intent, and nothing ever ran it against the code.

**Invariants 5 and 6 are now `scripts/web-only-and-unpriced.sh`, and writing it
found invariant 6 being violated rather than merely unenforced.**

The estate-wide removal of paid language took out the sender and left the
receiver. TokenFuse stopped emitting `plan_required` in its PR #142 on
2026-07-27; this console still carried an `UpsellBanner` component rendering
the word "upgrade" and a purchase URL, wired into two views, for a message no
current Cloud sends. In a public repository, anyone reading the source
concluded there was a paid tier.

The component is gone and `plan_required` now routes through the ordinary
error banner like every other kind. The variant is still PARSED, so a console
pointed at an older Cloud reports the refusal honestly instead of going blank;
what it no longer does is ask anybody to buy something.

Four present-tense references to the deleted shells went with it, including one
calling this browser build "this desktop build". References in the past tense,
recording that the shells existed and were removed, are left alone: that is
history, and it is worth keeping.

**Invariant 1** is now `scripts/no-cloud-credentials.sh`, and it came out
stricter than the note that asked for it. That note wanted credentials confined
to `connectors`; in fact no crate reads one at all, and none needs to, because
`cloud_cli.rs` spawns the operator's already-authenticated CLI. So the check
forbids reading one ANYWHERE rather than policing where it may live, which is a
line that cannot be argued down one crate at a time.

It also forbids declaring a provider SDK. That is the way this invariant would
actually be lost: not by somebody deciding to store keys, but by an operator
without the CLI installed, and an SDK looking pragmatic. Verified by breaking
both ways.

Invariants 2, 3 and 4 are the ones that most deserve tests rather than greps,
and invariant 4 in particular is the kind of promise that erodes one placeholder
at a time.

**Invariant 2** got those tests on 2026-08-05, and writing them found the
ceremony's two ends unguarded rather than merely untested: an enrolled passkey
could not be removed at all (a lost authenticator locked its owner out of all
five commands with no in-product way back), and a new one could be enrolled on
nothing but a session cookie, which is the exact credential the ceremony
exists to distrust. Both are fixed and held by tests. The third finding stays
visible in the marker: there was no way to make the ceremony mandatory, and
now there is, but it is opt-in, so the invariant is configuration-dependent
until a box sets it.

## Standing rule

An approved architecture decision is **not finished** until it is two things: a
numbered invariant in this file, and a gate in a script if it can be checked
structurally. Until then it is a document, and documents do not stop code.

## Conventions

- **No long dashes** anywhere: not in code, docs, commit messages, or PR
  bodies. Use a comma, a colon, parentheses, or a short hyphen.
- Nothing paid or metered gets enabled without telling the user first and
  getting agreement.
- Do not delete or revoke keys, tokens, or certificates on your own initiative.
