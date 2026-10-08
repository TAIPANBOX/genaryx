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
./scripts/one-test-binary-per-crate.sh  # invariant 18
./scripts/no-owner-quotes.sh         # invariant 21
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
   *(partly gated: `crates/core/tests/it/console_chain_test.rs` holds the chain
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
   truncation nobody could see: @measured `crates/api/tests/it/stats_scale.rs` at
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
   `crates/api/tests/it/quarantine_is_visible_test.rs` drives the REAL captured
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
   *(test: `crates/core/tests/it/conform_test.rs`,
   `a_v1_0_event_is_accepted_and_resolved`,
   `a_v1_0_claimed_subject_is_refused_under_one_reason`,
   `v0_3_stays_refused_by_version`,
   `the_vendored_v1_0_schema_widens_only_the_subject`;
   `crates/core/tests/it/ingest_test.rs`,
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

    A 200 status is not proof by itself, only vouchryx's own confirmed body
    is: `call_vouchryx` checks the parsed body for `"revoked":true` and, on a
    200 that does not confirm it (an HTML page from a `GENARYX_VOUCHRYX_URL`
    pointing at the wrong service, `{}`, `{"revoked":false}`, anything else),
    reports `DelegationError::Unconfirmed` (real `http_status` 200, journaled
    `verify_result` "200 without revoked:true confirmation", NEVER
    "revoked:true") rather than treating the status alone as success. Found
    in review of the first draft, 2026-09-24, before this invariant's first
    commit landed: the draft trusted any 200. The response body itself is
    read capped at 64 KiB (`MAX_RESPONSE_BYTES`, `read_capped_body`,
    incremental, not buffer-then-check), so the same wrong URL cannot make
    this console hold an arbitrary amount of memory reading whatever
    answered instead.

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
    sweeps it; `crates/api/tests/it/delegation_revoke_test.rs` (21 tests) drives
    the real function against a hand-rolled stub vouchryx for every outcome
    (200/400/401/403/503/unreachable), a 200 that does NOT confirm
    (`vouchryx_200_with_an_html_body_is_not_confirmed_as_success`,
    `_with_an_empty_object_`, `_with_revoked_false_`), a response body over
    the cap (`a_response_body_over_the_cap_is_refused_and_never_buffered_whole`),
    the exact posted body and bearer header, that a misconfigured pair
    refuses to start, that a bad argument never reaches the stub (a counting
    stub asserts zero connections), and that the bearer key never appears in
    the journaled line or the returned error; `crates/web/src/main.rs`'s
    `a_viewer_is_refused_by_the_role_gate_on_delegation_revoke`,
    `an_approver_is_refused_by_the_role_gate_on_delegation_revoke`,
    `delegation_revoke_with_a_passkey_and_no_assertion_is_428` and
    `delegation_revoke_with_a_valid_assertion_runs` hold the full HTTP-level
    gate. Red first as a compile failure on the unfixed tree (the module did
    not exist, and later the `Unconfirmed` variant did not); the HTTP-level
    gate tests were red on the unfixed tree for a different, real reason:
    `delegation_revoke` was not yet a dispatchable command at all, so a
    viewer's request 404'd rather than 403'd.

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
    both-subject-and-jti call reach the network); the `revoked == true` check
    replaced with a constant `true` caught by all three unconfirmed-200
    tests at once, each showing the same `verify_result: "revoked:true"` an
    HTML page or `{"revoked":false}` must never produce. Scenarios:
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
    box is what this control is for. The 64 KiB cap bounds what this
    console ACCUMULATES across `.chunk()` reads, checked before each chunk
    is appended; it does not bound the size of any single underlying TCP
    read reqwest itself performs, which is reqwest's own buffering and not
    something this code controls.)*

13. **Felyx's own model calls carry an agent identity, so a TokenFuse gateway
    enforcing policy can govern the console's own AI instead of refusing it.**
    `@decided 2026-09-27`, measured the same day on a live cluster: Felyx
    pointed at the stack's own gateway with its Wardryx hook in enforce mode
    got a `400 identity_required` on every call, because
    `crates/copilot/src/provider/{anthropic,openai}.rs` sent `x-api-key`/bearer
    auth and `x-fuse-run-id` (the C2 self-budget) but never `x-fuse-agent-id`.
    Both real provider clients now send `x-fuse-agent-id` beside
    `x-fuse-run-id` on every call. The value comes from
    `GENARYX_COPILOT_AGENT_ID` (`crates/api/src/copilot/state.rs`, read into
    `CopilotConfig::agent_id`, `crates/copilot/src/config.rs`): unset or empty
    resolves to `agent://<GENARYX_ORG_DOMAIN, default "local">/genaryx/felyx`,
    the same domain source `crates/api/src/journal.rs` already reads for the
    console's own emitted `agent_id` (a second independent reader, the
    sanctioned shape trap 13 already names for
    `TOKENFUSE_GATEWAY_ADMIN_KEY`/`GENARYX_SCAN_TARGET`); an explicit value
    that does not match the estate's agent-id grammar
    (`^agent://[a-z0-9.-]+/[a-z0-9._/-]+$`, `crates/core/src/schemas/
    agent-event.v0.2.schema.json`) is refused, naming the setting and the
    value, the same "say so, do not guess" posture every other misconfigured
    copilot setting already takes (`ConfigError`, surfaced as
    `CopilotInner::Failed`'s reason: Felyx reports itself disabled, the
    console keeps serving every other panel). The launchers are NOT changed
    by this: nothing sets `GENARYX_COPILOT_AGENT_ID` in a running stack yet,
    which is a separate, later decision.
    *(test: `crates/copilot/src/config.rs`'s
    `resolved_agent_id_defaults_explicit_empty_and_malformed` (default
    derivation, an explicit org domain, an explicit override, empty falling
    back, a malformed value refused, a value missing its path segment
    refused); `crates/copilot/tests/it/agent_id_header_test.rs`'s
    `anthropic_request_carries_the_configured_x_fuse_agent_id_header` and
    `openai_compat_request_carries_the_configured_x_fuse_agent_id_header`
    (a hand-rolled stub server capturing the real outbound request, both
    providers); `crates/api/src/copilot/state.rs`'s
    `config_from_env_reads_the_provider_surface` (the env var reaches the
    config type verbatim). Every one of these ran against the unfixed tree
    first: the config-level test failed to compile (11 errors: no `agent_id`
    field, no `resolved_agent_id` method, no `BadAgentId` variant); the header
    tests failed on an assertion (`x-fuse-agent-id` absent) with the header
    line removed; the state.rs test failed on an assertion (`None` where
    `Some("agent://acme.example/genaryx/felyx")` was expected) before
    `config_from_env` read the variable. Scenarios:
    `features/felyx-sends-its-own-agent-identity.feature`, four, each bound;
    gate: `scripts/features-are-bound.sh`.

    Measured 2026-09-27 on a three-node k3d cluster, stack-k8s v1.1.13 with
    TokenFuse v1.3.0: Felyx, pointed at the stack's own gateway by its
    ClusterIP (the residency gate accepts only a literal loopback/private
    address, not a service name) with Wardryx's policy hook in enforce and
    the console built as `genaryx-console:v1.1.14`, answered money, incident,
    identity-alert and approval-inbox questions with figures equal to the
    control plane's own, using its default agent id
    (`agent://local/genaryx/felyx`, no `GENARYX_ORG_DOMAIN` set). Where it
    still says nothing: no launcher sets `GENARYX_COPILOT_AGENT_ID` or wires
    a console at this gateway in a real deployment, this run patched one
    console by hand; and the default agent id used sits outside the stack's
    own trust domain, so a launcher wiring this up should also set
    `GENARYX_ORG_DOMAIN` or `GENARYX_COPILOT_AGENT_ID` to match it.)*

14. **The residency gate accepts a HOSTNAME only when it can prove every
    address that name resolves to is local, and only for a hostname the
    operator explicitly named, and that proof is re-run at connection time,
    not only once when the provider is built.**
    `@decided 2026-09-27`, measured the same day: the launchers route Felyx
    through the stack's own TokenFuse gateway by default, reached in
    Kubernetes as a Service name and in Compose as a service name on a
    private bridge network - neither a literal address nor `localhost`, the
    only two things `residency::is_local_endpoint` could prove local on its
    own, so the gate refused both outright. The only existing workaround,
    `GENARYX_COPILOT_ALLOW_REMOTE`, opens every destination, which defeats
    the gate's own purpose.

    `GENARYX_COPILOT_LOCAL_HOSTNAMES` is a comma-separated allow-list of
    exact hostnames (case-insensitive); empty, the default, keeps the gate's
    original behaviour exactly - any hostname other than `localhost` refused
    outright, no DNS call at all. Only a hostname the operator names is
    resolved and checked at all: every address it resolves to, right now,
    must be loopback/RFC1918/link-local (either IP family); one public
    address among several refuses the whole name, and an unresolvable name,
    or one resolving to nothing, is refused too, never treated as local by
    default. The SAME check is wired into the HTTP client itself as a custom
    DNS resolver, so it re-runs on every connection the client makes, not
    only the one build-time check the provider constructor performed: a name
    that resolves privately today and differently tomorrow (DNS rebinding,
    or a Service's backing pod changing) is caught at the moment reqwest
    actually dials, because the address checked is always the address
    handed to the connector. A refusal, whether caught at construction or at
    connection time, surfaces as the existing operator-readable
    `NonLocalEndpointRefused`, never a bare transport error: a residency
    refusal found in a failed request's error chain is promoted to that
    shape rather than left as an indistinguishable network failure. Literal
    IPs and `GENARYX_COPILOT_ALLOW_REMOTE` are untouched: neither hostname
    logic nor any DNS lookup runs on either of those paths. While the gate is
    in force the client follows no redirect and reads no proxy from the
    environment (`provider::residency_client`): a `Location` naming a literal
    public address, or an inherited `HTTP_PROXY`, would be a destination the
    gate never checked, since neither passes through the resolver.
    *(test: `crates/copilot/tests/it/residency_no_redirect_test.rs`'s
    `a_gated_client_on_a_literal_local_address_does_not_follow_a_redirect`,
    `a_gated_client_on_an_allow_listed_hostname_does_not_follow_a_redirect`;
    `crates/copilot/tests/residency_no_proxy_test.rs`'s
    `a_gated_client_ignores_the_proxy_the_environment_names`;
    `crates/copilot/src/resolver.rs`'s
    `a_name_resolving_only_to_private_addresses_is_accepted`,
    `a_name_resolving_to_one_private_and_one_public_address_is_refused`,
    `a_name_resolving_to_a_public_ipv6_address_is_refused`,
    `a_name_that_does_not_resolve_is_refused`,
    `a_name_resolving_to_no_addresses_is_refused`,
    `the_resolver_rechecks_on_every_call_and_catches_a_later_public_answer`,
    `counting_lookup_records_every_call`;
    `crates/copilot/src/residency.rs`'s
    `classify_host_distinguishes_a_hostname_from_a_literal`;
    `crates/copilot/tests/it/residency_hostname_test.rs`'s
    `a_hostname_resolving_only_to_a_private_address_is_accepted_end_to_end`,
    `a_hostname_not_on_the_allow_list_is_refused_with_no_dns_lookup_at_all`,
    `a_hostname_resolving_to_one_private_and_one_public_address_is_refused_at_construction`,
    `an_unresolvable_hostname_is_refused_at_construction`,
    `a_hostname_that_resolves_privately_at_build_and_publicly_at_request_is_refused_at_request_time`,
    `anthropic_hostname_path_also_rechecks_at_request_time`;
    `crates/api/src/copilot/state.rs`'s
    `config_from_env_reads_the_provider_surface` (extended: the new
    variable's comma-split/trim/empty-drop parsing). Every test above that
    names a NEW behaviour ran against the unfixed tree first: the module
    (`crates/copilot/src/resolver.rs`) did not exist, so every reference to
    `HostnameLookup`/`ResidencyDnsResolver`/`resolve_all_local` failed to
    compile, the same shape invariant 13's own red-first record took. The
    existing literal-IP and ALLOW_REMOTE tests
    (`refuses_public_by_default_allows_when_opted_in`,
    `allows_public_endpoint_when_opted_in`, `local_endpoint_needs_no_opt_in`,
    and their `openai.rs` equivalents) are unchanged and still pass, holding
    that this addition left both paths alone.

    Four mutants planted by hand in `crates/copilot/src/resolver.rs` and
    `provider/mod.rs`, each run red against its catching test and restored
    byte for byte: accepting a name if ANY of its addresses is local instead
    of requiring ALL, caught by
    `a_name_resolving_to_one_private_and_one_public_address_is_refused` and
    `a_hostname_resolving_to_one_private_and_one_public_address_is_refused_at_construction`;
    never wiring the connection-time resolver (checking only once, at build
    time), caught by
    `a_hostname_that_resolves_privately_at_build_and_publicly_at_request_is_refused_at_request_time`
    (without it, the real system resolver fails the fake hostname with an
    ordinary transport error rather than the readable residency refusal the
    test requires); treating an unresolvable or empty answer as local
    instead of refusing it, caught by `a_name_that_does_not_resolve_is_refused`,
    `a_name_resolving_to_no_addresses_is_refused`, and
    `an_unresolvable_hostname_is_refused_at_construction`; and skipping the
    check for IPv6 (any IPv6 address treated as local, unchecked), caught by
    `a_name_resolving_to_a_public_ipv6_address_is_refused`. Scenarios:
    `features/felyx-checks-a-hostname-at-connection-time.feature`, six, each
    bound; gate: `scripts/features-are-bound.sh`.

    Where it says nothing: no launcher sets `GENARYX_COPILOT_LOCAL_HOSTNAMES`
    in a running stack, so this closes the gap invariant 13 measured without
    itself wiring anything up - that remains a separate, later decision. The
    OS resolver (`SystemLookup`, via `ToSocketAddrs`) is the production
    lookup; every test here injects a fixed or sequenced answer table
    instead, so nothing above is proven against a real DNS server, a real
    Kubernetes Service, or a real Compose network - only against the
    resolver seam this gate is built on. `resolve_all_local` runs on a
    blocking task per connection attempt (matching hyper-util's own
    `GaiResolver`), and that scheduling overhead is not measured here.)*

15. **Every money value a copilot tool hands to the model is decimal USD
    under a key that says so, never a bare micro-USD integer.**
    `@decided 2026-09-27`, found the same day, console v1.1.17, forge lab:
    asked "Which planes can you see, and is each one healthy?", Felyx
    answered "Run `genaryx-copilot` exceeded its $50 budget by 27% ($63.64
    spent)". The Cloud's own figures were `budget_micros: 50000` (five
    cents) and `spent_microusd: 63640` (about six and a third cents), read
    straight off a `genaryx_connectors` DTO (`crates/copilot/src/tools/
    cloud.rs`'s `to_result`, a plain `serde_json::to_value` of the wire
    struct) and handed to the model as bare integers, with only the
    tool's own prose description, not the JSON itself, ever saying the unit
    was microdollars. In the SAME session, a different question about the
    SAME run's alert got the conversion right ("spent $0.0547 on a $0.05
    budget"): the model can do this arithmetic, it is just not required to,
    and a value that is only sometimes converted correctly is a defect even
    on the calls where it happens to come out right.

    Every affected tool (`money_summary`, `list_runs`, `list_agents`,
    `savings`, `alerts` in `crates/copilot/src/tools/cloud.rs`;
    `savings_breakdown`, `cost_per_action` in `crates/copilot/src/tools/
    optimize.rs`) now passes its result through `dollarize`
    (`crates/copilot/src/tools/mod.rs`) before returning it: every
    `..._microusd` key becomes a `..._usd` decimal sibling, and the one
    field in this whole contract that means microUSD without saying "usd"
    in its own name, `Alert`/`BudgetResponse`'s `budget_micros`, is
    converted by an explicit allow-list entry rather than a bare `_micros`
    suffix match, so a future unrelated `..._micros` field (a duration, say)
    is never misread as money. A nested reason -> amount map
    (`TokenfuseSavings::by_reason_microusd`) is walked one level further,
    since there the unit lives in the outer key, not a sibling field, and a
    `null` amount (`cost_per_tool_call_microusd: None`, "the rate is not
    known") stays `null` under the renamed key rather than becoming a
    computed `0.0` a model could read as a real answer. The
    `genaryx_connectors` wire DTOs themselves are untouched, staying
    byte-exact mirrors of the Cloud's own `store.rs` shapes
    (`crates/connectors/src/cloud_rest.rs`'s own doc); the conversion runs on
    the JSON `to_result` already built, the same "wire DTO stays wire-exact,
    a separate value carries the conversion" split `crates/api/src/money/
    commands.rs` already draws for the web frontend (its own
    `micros_to_usd`, `dollarize`'s sibling on the other side of a crate
    boundary this crate cannot depend across).
    *(test: `crates/copilot/src/tools/mod.rs`'s eight `dollarize_*`/
    `the_real_defect_amounts_*` tests hold the shared conversion itself,
    including the allow-list (not suffix) rule and the null-stays-null rule;
    `crates/copilot/src/tools/cloud.rs`'s
    `money_summary_dollarizes_its_spend_field`,
    `list_runs_dollarizes_every_row_and_the_wrapping_total`,
    `alerts_dollarizes_both_the_spent_and_the_ambiguously_named_budget_field`
    and `crates/copilot/src/tools/optimize.rs`'s
    `savings_breakdown_dollarizes_its_money_fields`,
    `cost_per_action_dollarizes_its_money_fields_and_keeps_a_null_rate_null`
    hold each tool's own exact shape against a constructed connector DTO, no
    live plane needed; `crates/copilot/tests/
    money_reaches_the_model_as_usd_test.rs`'s five tests drive the REAL
    `ToolRegistry::dispatch` path the agent loop uses, against a hand-rolled
    stub Cloud (the same shape `crates/copilot/tests/
    agent_id_header_test.rs` and `crates/api/tests/it/delegation_revoke_test.rs`
    already use) - the layer the in-module tests do NOT cover, and the one
    that actually caught the mutant below. Every test here failed to compile
    against the unfixed tree (`dollarize` did not exist).
    Mutant (planted by hand, run red, restored): the `read_tool!` macro's
    call to `dollarize` dropped, reverting `money_summary`/`list_agents`/
    `savings`/`alerts` to the raw connector JSON. Every in-module unit test
    in `cloud.rs` and `optimize.rs` still passed, because they call
    `to_result`/`dollarize` directly rather than through the tool the model
    actually calls; only `money_reaches_the_model_as_usd_test.rs`'s
    `ToolRegistry::dispatch` tests caught it (4 of its 5 failed, each on the
    exact assertion the dropped conversion breaks). That gap is the reason
    the integration test file exists at all rather than stopping at the
    in-module tests. Scenarios: `features/felyx-reads-money-as-usd.feature`,
    six, each bound; gate: `scripts/features-are-bound.sh`.

    Where it says nothing: no live model rerun against a real Cloud proves
    Felyx itself now answers the three demo questions correctly; this closes
    the tool-output half of the defect (the model can no longer receive a
    bare micro-USD integer from these tools), not a re-run of the exact
    2026-09-27 conversation. `incidents` carries no money field today and is
    unchanged; a money field added to it later must still be wrapped in
    `dollarize` by hand; nothing here makes that automatic.)*

16. **Felyx states a budget only when a tool returned it.**
    `@decided 2026-10-05`, measured the same day on the forge lab
    (genaryx-console v1.1.19, Haiku 4.5 through the stack's own gateway):
    asked about the home router agents, Felyx said p1-beryl2 "exceeded
    budget alert (spent $0.004019 vs $0.001 cap)" although no budget existed
    on any beryl2 run (the cap was p1-brume's), and asked which runs have a
    budget it called mig-flint "no budget" although `GET /v1/budgets` held
    4500 uUSD for it. There was no budgets tool, so it read budgets off
    `alerts`, which lists a run only once it is near or over its limit.

    Three parts, each held separately. A read-only `budgets` tool
    (`crates/copilot/src/tools/budgets.rs`) joins `GET /v1/budgets` with each
    run's spend from `GET /v1/runs`, lists the runs with spend and no budget,
    and adds unit budgets from `GET /v1/unit-budgets` with `GET /v1/units`'
    month-to-date spend; a spend the Cloud has no record of is `null`, never
    zero, and a Cloud without `/v1/unit-budgets` is reported as unable to say,
    never as having none (`CloudClient::{budgets,unit_budgets,units}`). The
    system prompt says budgets come from `budgets`, never from `alerts`. And
    the agent loop checks every draft answer against that answer's own tool
    results (`crates/copilot/src/grounding.rs`): a clause naming one run with
    a budget no tool returned, a clause calling a budgeted run unbudgeted, or
    budget facts stated while `budgets` was available and not called, sends
    the draft back once with the findings; a finding that survives is
    appended to the answer as a note and listed in `Answer::unsupported_claims`.
    The clause heuristic and the single revision turn are `@claude` design
    choices, open to reversal; the requirement above is the decision.
    The no-signer, propose-only shape is unchanged: the tool issues GETs only
    and is not a propose tool, and a propose tool's result is never counted
    as a budget fact.
    *(test: `crates/copilot/tests/it/felyx_reads_budgets_test.rs`, fourteen,
    on a stub Cloud carrying the forge figures; `crates/copilot/src/
    grounding.rs`'s seven, including a 200-seed hostile-text sweep;
    `crates/copilot/src/tools/budgets.rs`'s three. Red first on the unfixed
    tree: twelve of the fourteen failed on assertions (no `budgets` tool, no
    revision turn, no prompt rule) with the two `unsupported_claims` lines
    removed so the file compiled; the two that passed are the no-false-alarm
    cases, green by design. Seven mutants planted by hand, each caught and
    restored: the single-run budget check removed, the no-budget phrasing
    never recognised, the budgets-unread rule dropped, the revision turn
    skipped, run spend not joined, at-or-over made strict, the residual note
    dropped. Scenarios:
    `features/felyx-reads-budgets-from-the-budgets-tool.feature`, nine, each
    bound; gate: `scripts/features-are-bound.sh`.

    The second forge run, the same day on `genaryx-console:v1.1.24`, got
    every run right, matched `GET /v1/budgets` exactly on "which runs have a
    budget", and still closed with "Flint and brume have budget overages":
    no flint run was over, and "flint" is an agent, not a run id. So the
    check also reads OVER claims ("over", "overages", "exceeded"...): a
    clause calling one run over its budget while the tools returned it
    under, or naming an agent (by full id or the last segment of it) none of
    whose runs a tool returned at or over its budget, is sent back too; "no
    budget issues" is no longer read as "no budget"; and the revision
    request asks for a fresh answer, after the first run showed the
    operator "You're right. Let me revise".
    *(test: four more in the integration file, the live forge rows as a
    no-false-alarm fixture; three more in `grounding.rs`. Three of the new
    integration tests were red on `1a38148`. Five more mutants caught: the
    agent check, the run-over check, the "no budget issues" exemption, an
    alert's fraction ignored, the fresh-answer instruction dropped.
    Scenarios: two more, eleven in all.)*

    Where it says nothing: the answer check is a clause-level text heuristic,
    not a parser. It does not check a positive budget claim in a clause that
    names several runs, a run id no tool returned, budget AMOUNTS, or unit
    budgets; a negative claim separated from its runs by a semicolon escapes
    it, and an over claim about several runs in one clause is not checked
    run by run (agents are). A budget a gateway applies on its own (a per-run default ceiling, an
    identity-map unit cap) is invisible to the Cloud and so to this tool,
    which says so in its own result. The Money panel's runs table read
    budgets from `alerts` until invariant 17 moved it to the same
    `GET /v1/budgets` read.)*

17. **The Money panel's runs table shows every run budget the Cloud holds, and
    a budget it could not read is never shown as "no budget".**
    `@decided 2026-10-05`, measured the same day on the forge lab: the table
    read a run's budget only from `GET /v1/alerts` plus the budgets set in the
    current console session (`MoneyState::budget_overrides`). `/v1/alerts`
    lists a run only once it is near or over its limit, so mig-flint (4500 uUSD
    budget, 3210 spent) showed no budget and the table printed "no cap": a
    real budget under the label of an absence, invariant 8's shape.

    `money_runs` (`crates/api/src/money/commands.rs`) now joins `GET /v1/runs`
    with `CloudClient::budgets()` (`GET /v1/budgets`), and that map is the only
    budget source while it answers; `alerts` is not read at all then. The
    session overrides are removed: the Cloud's map holds a budget the moment
    `set_budget` returns, and an override could outlive a change made
    anywhere else. A budget map that cannot be read (any status, a transport
    error, a body that is not a run -> micros map) does not cost the table:
    the runs are still listed, `GET /v1/alerts` supplies the budgets it knows,
    one stderr line names the failure, and every `RunDto` carries
    `budgets_read: false`. The web views read that flag through one helper
    (`apps/web/src/lib/moneyExport.ts`'s `budgetUnknown`): the runs board
    prints "cap unknown" with a hover naming the failed read, Agent 360
    "unknown", Incident 360 "budget could not be read", and only a read map
    earns "no cap"; the runs export carries a `budgets_read` column beside
    `budget_usd` and its caveat says what an empty cell means on each. The
    flag is per row so `Vec<RunDto>` keeps the shape every existing view
    reads. The mock preview stands in for a box whose map answered. The
    requirement (the map is the source, and "none" is kept apart from
    "could not read") is the decision; the per-row flag, the fallback to
    `alerts` and the wording are `@claude` design choices, open to reversal.
    `GET /v1/budgets` has existed since tokenfuse #64, so a Cloud without the
    route is the rare case; one that refuses or is unreachable is the likely
    one.
    *(test: `crates/api/tests/it/money_runs_budgets_test.rs`, seven, on a stub
    Cloud carrying the forge figures: a budgeted run with no alert, a run with
    none, the map winning over an alert figure, 404/500/403 on the map,
    neither map nor alerts answering, runs failing as an error, and ten named
    hostile bodies plus a 200-seed sweep. Red first on the unfixed tree: the
    file failed to compile (no `budgets_read`), and with the flag swapped for
    an existing field five of the seven failed on assertions, the defect test
    as `left: None, right: Some(0.0045)`. Two mutants planted in
    `money_runs` and restored, each caught by three tests: an unread map
    reported as read, and an unread map failing the whole table.
    `apps/web/src/lib/moneyExport.test.ts`'s six board and export cases,
    three of them red against the unfixed `RunsBoard.tsx`/`moneyExport.ts`.
    Scenarios: `features/money-runs-show-every-cloud-budget.feature`, five,
    each bound; gate: `scripts/features-are-bound.sh`.

    Where it says nothing: a budget a gateway applies on its own (a per-run
    default ceiling, `x-fuse-budget-usd` sent by the caller) never reaches
    the Cloud's map, so the table cannot show it, the same limit invariant 16
    names. Unit budgets are not in this table. No live console has been
    pointed at the forge Cloud with this build; the figures above come from a
    stub carrying the forge numbers.)*

18. **Each crate's integration tests link into one test binary.** Cargo makes
    every top-level `tests/*.rs` its own executable, each statically linking the
    crate and its whole dependency tree, so one `cargo test` after a change
    relinks all of them and writes one binary's size times the file count.
    `@claude 2026-10-06`, measured the cost in a sibling repo in this estate:
    about forty such binaries of 200 MB, about 8 GB written per run, several TB
    to one SSD in a week. Each file lives at `tests/it/<file>.rs`, declared
    once in `tests/it/main.rs`, test names
    unchanged (`features-are-bound.sh` binds to them), and a single file runs as
    `cargo test -p <crate> --test it <file>`. `[profile.dev]` in the root
    `Cargo.toml` carries line tables only for this workspace and no debug info
    for dependencies, which tests inherit; a panic still names its file and line.

    One binary is one process, so the merge was audited for state the files
    used to keep apart (environment writes, statics, scratch paths, ports,
    global subscribers). One file stays alone, allow-listed with its reason:
    `crates/copilot/tests/residency_no_proxy_test.rs` sets `HTTP_PROXY` for the
    whole process, which every other reqwest client in that crate's tests
    would read.
    *(gate: `scripts/one-test-binary-per-crate.sh`, which counts top-level
    `tests/*.rs` files and `[[test]]` tables per crate and fails on a second one
    outside its allow-list, and on an allow-list entry whose file is gone; four
    cases in `gates-have-teeth.sh`: a planted second file, the allow-listed file
    removed, a new module inside `tests/it/` that must NOT fire, and no crates
    at all, which must say it measured nothing. Not held: that a merged file
    does not race another one in the same process; the audit was by reading,
    and a later test that writes process-wide state is caught only if it fails.)*

19. **An identity refusal is filed under the key that made the call, never
    under the agent it claimed.** TokenFuse's `identity_mismatch` puts the
    CLAIMED agent on the envelope's `agent_id` and the credential that
    actually called in `data.key_id` (its strict-identity enforce path), so
    the envelope names the one agent known NOT to have made the call. Found
    reading the code on 2026-10-08, against the home-lab run of 2026-10-07
    behind TokenFuse's invariant 81 (a key `forge-imposter` claiming
    `agent://taipanbox.dev/routers/flint`): this console read the envelope
    like any other event and charged the attempt to flint four times over,
    as a stop in the Statistics counts, on flint's Agent 360 stop list (and
    its profile), as the subject of a high-severity incident, and among
    flint's own calls in Felyx's `cost_per_action`.

    `@decided 2026-10-08`: filed under `key:<key_id>`, the rule TokenFuse
    applies to its own FOCUS export; under `key:(none)` when the event names
    no readable key (a non-empty string is the only thing that names one),
    never back under the claim. The `key:` prefix keeps a key named like an
    agent out of that agent's figures. One rule, three spellings, each held:
    `genaryx_core::attribution::filed_under` (Rust),
    `attribution::FILED_UNDER_SQL` (the store's aggregate, the detail read,
    and the per-subject profile filter, built from it rather than restated),
    and `apps/web/src/lib/attribution.ts` (the incident centre, Incident 360,
    the export link, the Statistics table). The incident leads with "claimed
    <agent> with key <key>" instead of the agent, groups by key, and Incident
    360 shows no agent card or owner for a key. `PER_AGENT_COST_QUERY` re-keys
    the trace row the same way (`decision = 'identity_mismatch'` and
    `key_id`, both in TokenFuse's trace read schema since before
    `tool_calls`, which the query already needed). The demo's simulated bus
    carries one such refusal and the demo Statistics a `key:forge-imposter`
    row.
    *(test: `crates/core/tests/it/store_test.rs`'s
    `an_identity_refusal_is_grouped_under_its_key_in_the_aggregate` (the
    aggregate and the profile, keys null, empty, a number, absent, SQL-shaped
    and agent-shaped) and
    `an_identity_refusal_is_filed_the_same_way_in_sql_and_in_rust` (a
    200-seed sweep of hostile `key_id` shapes through both spellings and the
    aggregate); `crates/core/src/attribution.rs`'s two;
    `crates/api/src/stats/mod.rs`'s
    `an_identity_refusal_is_counted_under_the_key_not_the_claimed_agent` and
    `an_identity_refusal_is_not_on_the_claimed_agents_stop_list`;
    `crates/connectors/src/tokenfuse.rs`'s
    `an_identity_refusal_is_filed_under_its_key_in_cost_per_action` (the
    exact query text, run on SQLite); `apps/web/src/lib/incidents.test.ts`'s
    five, `incidentExport.test.ts`'s one, `attribution.test.ts`'s three.
    Red first: the test-only commit of genaryx#93 failed in CI on the stats,
    store and connector tests and on five of the six incident tests (the
    sixth, every other event keeping its own agent, is a control and green by
    design). Scenarios:
    `features/an-identity-refusal-is-filed-under-its-key.feature`, six, each
    bound; `scripts/features-are-bound.sh` now also binds a scenario to a web
    test by its exact title. Gate: four product mutants and one non-fault in
    `scripts/gates-have-teeth.sh`, run through cargo in CI (the aggregate
    grouping by envelope, the stop list reading the envelope, a keyless
    refusal falling back to the claim, the cost query not re-keying), plus a
    case for a web binding whose test is gone. Three web mutants planted by
    hand and caught: the incident group key, the incident subject and the
    claim sentence each put back on the envelope.

    Where it says nothing: Agent 360's raw event list (`agent_events`) still
    shows the refusal on the claimed agent's feed, as the bus line it is,
    under its own type; the per-model cost row still counts the refused calls
    as calls of the model they named; in TokenFuse's `warn` mode a mismatched
    call is forwarded and its trace row says `allow` with the claimed id, so
    nothing here can tell; and a key subject has no card of its own yet.)*

20. **Felyx counts a reasoning model's output the way the provider bills it.**
    `crates/copilot/src/provider/openai.rs` read `completion_tokens` alone.
    Google's OpenAI-compatible endpoint leaves a thinking model's reasoning
    out of that count and bills it at the output rate (measured 2026-10-07 on
    Vertex AI, `gemini-2.5-flash`: prompt 14, completion 59, reasoning 560,
    total 633), so Felyx showed about a tenth of the output a TokenFuse
    gateway in front now charges. `@decided 2026-10-08`: output is the larger
    of `completion_tokens` and `total_tokens` less `prompt_tokens`
    (`usage_from`), the rule of TokenFuse's invariant 80 and CostCrew's #110.
    OpenAI's completion count already holds its reasoning, so there nothing
    changes and the reasoning detail is never added on top; a short total
    never lowers the output below the completion count; a total with no
    prompt count is output whole. Counts past `u32` saturate instead of
    wrapping (they wrapped before, a huge count reading as a small one).
    *(test: `crates/copilot/src/provider/openai.rs`'s
    `a_reasoning_model_usage_counts_its_reasoning_as_output`,
    `an_openai_shaped_usage_is_read_unchanged`,
    `a_short_total_never_lowers_the_output_below_the_completion_count`,
    `a_total_with_no_prompt_count_is_shown_as_output`,
    `hostile_usage_figures_saturate_and_never_wrap` (a 200-seed sweep against
    an independent statement of the rule). Red first in CI on the test-only
    commit of genaryx#94: three failed (59 against 619; 5 against 40; a
    wrapped 1 against a saturated limit), the two controls green by design.
    Scenarios: `features/felyx-shows-the-output-a-provider-bills.feature`,
    four, each bound; one case in `scripts/gates-have-teeth.sh` reads the
    completion count alone again and requires the Vertex test to fail.

    Where it says nothing: a Google-shaped usage with no `total_tokens` still
    shows the completion count alone; the Anthropic client is unchanged (its
    `output_tokens` already counts thinking); and this is the count Felyx
    shows and logs, not a charge: the gateway's settlement is the money.)*

21. **No tracked file quotes the owner or names him as the one who said,
    asked or decided something.** This repository is public. A decision is
    still recorded, because a later reader must know it is a decision and not
    something to re-derive: it is written as `@decided YYYY-MM-DD` followed by
    a paraphrase in English, and never edited afterwards. What is not written
    is his own wording, a provenance marker carrying his name, or his name as
    the one who decided. The owner as copyright holder, author or maintainer
    is not a quote and is allowed. `@decided 2026-10-08`: the estate's rule for
    public repositories, applied here.

    Until 2026-10-08 this tree carried 8 provenance markers with his first
    name in 7 files, 71 lines naming him in 33 files (most as a name and a
    date beside a decision), about 90 lines of Ukrainian prose (a whole
    follow-up note under `live-campaign/docs/`, the Phase labels in
    `docs/PHASE*.md` and in two test headers, and his quotes in web
    comments), and nothing that would stop the next one. Each was rewritten:
    a name and a date became `@decided` and the date, a quote became an
    English paraphrase without quotation marks, an operational duty became
    "the operator" or "the owner", the note was translated, and the Phase
    labels were renamed. Git history keeps the old text; rewriting it is not
    part of this.
    *(gate: `scripts/no-owner-quotes.sh`, in CI's web-ui job, over every
    tracked text file: the old marker, a guillemet beside Cyrillic, his first
    name capitalised outside an authorship line, and Cyrillic outside a string
    literal in a code file or anywhere in a prose file (a `testdata/` path is
    exempt). Red first: 161 findings on the tree before this change, 0 after.
    Six cases in `scripts/gates-have-teeth.sh`: the marker, the name as the
    one who decided, a Ukrainian quote in a Rust comment and a guillemet
    quote in a doc must each fail; the owner as copyright holder and Cyrillic
    inside a Rust string literal must each pass.

    Where it says nothing: an English quote of his words in ordinary
    quotation marks, an attribution that does not use his name ("the owner
    said"), and a paraphrase that is in fact a translation are prose a reader
    judges, and nothing mechanical can; a quoted Ukrainian phrase in a comment
    trailing code on the same line passes, since only a line that opens as a
    comment is read as prose whole; and it reads only what git tracks.)*

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
