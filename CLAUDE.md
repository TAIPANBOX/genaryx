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
   change, approval and the two operator WireGuard commands (issue a peer,
   revoke a peer) each need a fresh passkey confirmation. Five, not three:
   `crates/web/src/main.rs`'s `SENSITIVE_COMMANDS` is the list, and this file
   said three until 2026-08-05. Not a session, not a role check alone: the
   ceremony is per action, because the whole point is that a stolen session
   cannot pull the switch.
   *(partly gated: router-level tests in `crates/web/src/main.rs` drive the
   real axum router through the whole ceremony, and hold that an enrolled
   caller is refused without an assertion, that the command and argument
   bindings are enforced, that enrolling and removing a passkey each need a
   factor the session does not carry, and that with
   `GENARYX_WEB_REQUIRE_PASSKEY=1` all five are refused when nobody is
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
   disagree about the budget field names. What is NOT held is the sentence's
   scope: these four cover the Statistics panel, and nothing structural stops
   the next capped read elsewhere in the console from doing the same thing.)*

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
