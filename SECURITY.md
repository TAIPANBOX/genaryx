# Security Policy

Genaryx is a control room over an agent-governance stack. It holds admin
bearers for the money and policy planes, runs a break-glass kill switch behind
a passkey ceremony, and issues the WireGuard peer configs that are the only way
into the plane it manages. A bug here is not a display bug; it is a way to stop
an agent that should not have been stopped, or to fail to stop one that should.

## Reporting a vulnerability

Please report security issues privately, not in public issues or PRs:

- Open a **GitHub private security advisory**:
  <https://github.com/TAIPANBOX/genaryx/security/advisories/new>

Include the affected commit, a description, and a minimal reproduction. We aim
to acknowledge within a few days and to fix high-severity issues before any
public disclosure. There is no bug-bounty program; we credit reporters in the
advisory unless you prefer otherwise.

## Supported versions

Genaryx is pre-1.0; only `main` is supported. Fixes land on `main` and are not
backported.

## What this console assumes, so you know what is in scope

These are properties of the design rather than bugs. They are written down
because an assumption nobody stated is the one that bites.

- **The console is bound to loopback on its own box.** It is reached over a
  WireGuard tunnel the box itself issues. It is not built to face the internet,
  and publishing it directly is outside the threat model it was designed for.
- **A passkey confirms a destructive action; it does not authenticate the
  session.** Sign-in is a password against an Argon2id hash. The per-action
  WebAuthn ceremony is bound to the exact command and its exact arguments, and
  it exists so a stolen session cannot mint a kill. Where no passkey is
  enrolled, the action still proceeds and is journaled software-signed and
  labelled as such; that is deliberate, and it is a weaker state. Set
  `GENARYX_WEB_REQUIRE_PASSKEY=1` to refuse the action instead. Enrolling a
  passkey and removing one are part of the same control and neither rides on
  the session: the operator password is the factor for the first enrollment
  and the last removal, an assertion from an enrolled key for everything
  between.
- **The copilot has no signing key at all.** The copilot crate carries no
  dependency on the signing crate and a build-time test asserts it stays that
  way. If you find a path by which a model proposal becomes an executed command
  without a human ceremony, that is a vulnerability and we want to hear about it
  immediately.
- **The audit chain proves integrity, not honesty.** Every event carries the
  SHA-256 of the one before it over RFC 8785 canonical JSON. That shows a
  journal has not been altered after the fact. It cannot show that what was
  written was true when it was written.

## History note

This repository was published on 2026-07-27 with its history intact except for
five files removed before publication: two screenshots containing a pairing QR
code, and three screen recordings of a discontinued mobile client. The QR
carried a relay address that no longer exists and two one-time codes that
expired 274 seconds after they were minted on 2026-07-20. No credential, key or
certificate has ever been committed to this repository; that was verified
across every commit before publication.
