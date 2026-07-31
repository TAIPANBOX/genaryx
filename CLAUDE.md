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

## Hard invariants

Each one carries how it is held today. Use `(gate: ...)`, `(test: ...)`,
`(partly gated: ...)` or `(not enforced)`, and use the weakest one that is
true. An invariant with no check, written as though it had one, is worse than
an absent invariant.

1. **The console never stores cloud credentials.** Multi-cloud inventory is
   read-only and runs through the operator's own CLI, already authenticated on
   their machine. A console that holds cloud keys is a target, and it changes
   what this product is. *(not enforced)*
2. **A sensitive command requires a per-action ceremony.** Kill, budget change
   and approval each need a fresh passkey confirmation. Not a session, not a
   role check alone: the ceremony is per action, because the whole point is that
   a stolen session cannot pull the switch. *(not enforced)*
3. **Every privileged action is journaled into a verified hash chain.** An
   action that happened without a chain entry is indistinguishable from one that
   did not happen. *(not enforced)*
4. **The console shows the operator's real records, never a mock.** A card with
   no data says it has no data. Inventing a plausible number to fill a panel is
   the single worst thing this product can do, because the entire proposition is
   that what you see is what happened. *(not enforced)*
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
**Held by this file alone: invariants 1, 2, 3 and 4.**

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

**Invariant 1** remains the next checkable one: fail if any crate outside
`connectors` reads a cloud credential environment variable, and if `connectors`
persists one anywhere.

Invariants 2, 3 and 4 are the ones that most deserve tests rather than greps,
and invariant 4 in particular is the kind of promise that erodes one placeholder
at a time.

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
