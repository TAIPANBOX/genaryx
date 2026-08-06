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

## Decisions that have no gate yet

This list is debt, and it is here to stay visible rather than to be tidy.
**No invariant is now held by this file alone.** Every one of the six carries
a gate or a test, and the three that are partial say in their own marker which
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
