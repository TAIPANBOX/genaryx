# Console IdP, roles, and named audit actors (D15/B3, part 1)

Status: part 1 built on branch `feat/console-idp-roles`. Design record and
the exact contract. Part 2 (the WebAuthn per-action ceremony) is ALSO built,
on branch `feat/webauthn-per-action`, documented at the end of this file.

Defensive intent: this is about knowing WHO drove a privileged console action
and constraining who may drive one. It only ever reads the customer's own
IdP tokens they hand the box; nothing here reaches outside the perimeter (the
web console runs on the customer's box, docs/WEB-SHELL.md).

## The gap this closes

Today `genaryx-web` has one local operator account (Argon2id), and a session
just says "signed in". Two problems:

1. **The audit trail is not named.** A privileged mutation records its actor
   as `user://<org>/<OS-user>` - derived from the `USER` env var of the
   process running `genaryx-web` (`crates/api/src/money/state.rs`
   `operator_principal`), NOT from whoever logged into the browser. So the
   commands_journal and the `console_command` bus event attribute the kill to
   the box's service account, not to the human. A bank cannot answer "who
   killed it" from that.
2. **There are no roles.** Anyone who can sign in can do anything: read,
   grant a Wardryx approval, kill a run, move a budget.

Part 1 fixes both, without a live IdP (offline JWKS, like tokenfuse's
`crates/cloud/src/oidc.rs`) and without touching the desktop shell's
behavior.

## What part 1 delivers

1. **OIDC offline login** in `genaryx-web`, alongside the local account.
   The customer hands the box a static JWKS from their IdP (Entra ID for the
   first pilots, Q4); a browser signs in with an OIDC ID-token instead of a
   password. No `.well-known` fetch, no network: air-gap safe, byte-identical
   to tokenfuse's offline OIDC. Off unless configured; the local Argon2id
   account always stays as the break-glass owner.
2. **Three roles**: `viewer` (read everything), `approver` (+ grant/deny
   Wardryx approvals), `admin` (+ every privileged mutation: kill, budget,
   ack, remote ops, onboard, copilot-to-signed). Role gating happens at the
   `genaryx-web` command chokepoint (a 403 before the command runs), so it is
   one place, not sprinkled across planes.
3. **Named audit actors**: the signed-in principal flows into every
   web-originated `CommandRecord.operator` - `user://<org>/<sub>` for an OIDC
   user, `user://<org>/<username>` for the local account - so the journal and
   the bus name the human. The desktop shell is unchanged (still the OS user).

## Login contract (genaryx-web)

`GET /auth/session` gains `role` and `method`:

```json
{ "configured": true, "signed_in": true, "user": "alice",
  "role": "admin", "method": "local",
  "oidc_available": true }
```

- `oidc_available`: whether an OIDC config is present, so the browser shows
  the "Sign in with your organization" option.
- `role`: the caller's role this session (`viewer`|`approver`|`admin`).
- `method`: `local` | `oidc`.

`POST /auth/login` accepts either shape:
- `{ "username", "password" }` - the existing local path; the local account
  is `admin` (it is the box owner's break-glass credential).
- `{ "id_token": "<JWT>" }` - the OIDC path (only when configured). The token
  is verified offline; on success a session is minted for the mapped user and
  role. The raw token is never stored, never logged (it is a bearer secret).

### OIDC verification (mirrors tokenfuse/crates/cloud/src/oidc.rs exactly)

Config from env, all three required or OIDC stays off:
- `GENARYX_WEB_OIDC_ISSUER`, `GENARYX_WEB_OIDC_AUDIENCE`,
  `GENARYX_WEB_OIDC_JWKS` (inline JSON or a file path; static, never fetched).
Optional: `GENARYX_WEB_OIDC_SUB_CLAIM` (default `sub`),
`GENARYX_WEB_OIDC_ROLES_CLAIM` (default `roles`),
`GENARYX_WEB_OIDC_ADMIN_ROLE` (default `genaryx-admin`),
`GENARYX_WEB_OIDC_APPROVER_ROLE` (default `genaryx-approver`).

Checks, any failure => reject: well-formed JWS with `kid`; `kid` in the JWKS;
signature verified with algorithms derived from the JWK key type (never the
token header - closes RS256->HS256 alg-confusion); `exp`/`iss`/`aud` present
and valid (`set_required_spec_claims`); `sub` present and non-empty. Role:
`admin` if the roles claim contains the admin role, else `approver` if it
contains the approver role, else `viewer` (least privilege).

## Role gating (the command chokepoint)

`genaryx-web`'s `command()` handler resolves the session's role, looks up the
command's required role, and returns `403 {"error":"role <x> required"}`
before dispatching if the caller is below it. Classification
(`crates/web/src/roles.rs`):

- **viewer**: everything not listed below (all `*_status`, `*_list_*`, reads,
  `onboard_status`, `copilot_ask`/`copilot_explain`/`copilot_status`, the bus,
  and the read-only crypto inspections `crypto_scan_*` / `crypto_status` /
  `crypto_verify_evidence`).
- **approver**: `policy_decide_approval`.
- **admin**: `money_kill_run`, `money_set_budget`, `money_ack_incident`,
  `identity_rescan`, `drills_run`, `evidence_build`, `memory_forget`,
  `onboard_generate`, `onboard_write_passport`, `copilot_log_proposal_approved`,
  `pocket_connect`, `pocket_disconnect`, and the remote mutations
  (`remote_set_environment`, `remote_wg_connect`/`disconnect`,
  `remote_ssh_read_file`, `remote_ssh_tail_start`/`stop`). The single source
  of truth is `crates/web/src/roles.rs`, whose test asserts every dispatch
  command is classified; this list is prose that mirrors it.

A default-deny fallback: an unknown command name requires `admin` (fail
closed; unknown commands already 404 in dispatch, but the gate must not be
the thing that fails open). The classification is data, unit-tested against
the actual dispatch arm list so a new command cannot be added without being
placed.

## Named actor (how the session identity reaches the shared command layer)

The two command bodies that journal a mutation (`money`, `policy` in
`genaryx-api`) build `CommandRecord.operator` from `client.operator`, fixed
at bootstrap to the OS-user principal. To attribute a web mutation to the
signed-in human without churning every command signature across both shells,
`genaryx-api` gains a request-scoped override: a tokio task-local
`console_actor::ACTOR`. `genaryx-web` sets it (the session's principal) around
each `dispatch` call; the `CommandRecord.operator` builders read it as an
override of `client.operator`, falling back to the client's default when
unset. The desktop shell never sets it, so its behavior is byte-identical.
This is the one small edge added to the shared crate; it is request-scoped
context, the textbook task-local use, and it is documented and tested on both
paths (set => named actor; unset => OS-user default).

## Honest limits (stated, not buried)

- No `.well-known`/JWKS rotation fetch: the JWKS is static (air-gap by
  design). Rotating keys means updating the env/file, same as tokenfuse.
- No SAML/SCIM, no session-token refresh: an OIDC login mints a normal
  console session (12h idle), it does not track the IdP token's own lifetime
  beyond the one-time verification. A revoked IdP user keeps their console
  session until it idles out or the process restarts (restart cuts all
  sessions, the same control the local account has).
- Roles gate the console's command surface; they are NOT a second signature
  by themselves. Part 2 (WebAuthn, below) re-signs all five sensitive
  commands with a hardware-backed passkey once the operator has enrolled one;
  a caller with none enrolled still has the part-1 exposure only - "session
  opens the console" (docs/WEB-SHELL.md), narrowed by role - the documented
  trial fallback, not a bypass. `GENARYX_WEB_REQUIRE_PASSKEY=1` removes that
  fallback for a box that wants the ceremony unconditional.

## Part 2: WebAuthn per-action ceremony (built, branch `feat/webauthn-per-action`)

Signing in gets you the console; it does not get you the kill. FIVE
privileged commands carry the ceremony today
(`crates/web/src/main.rs`'s `SENSITIVE_COMMANDS`): `money_kill_run`,
`money_set_budget`, `policy_decide_approval`, `remote_operator_wg_config` and
`remote_operator_wg_revoke`. The last two joined because issuing a WireGuard
peer hands out a road into the control plane and revoking one takes an
operator's access away mid-incident. (This document, the README and CLAUDE.md
all said "three" until 2026-08-05, which is what a hand-copied list does.)
Each additionally requires a fresh, per-action WebAuthn assertion once the
caller has enrolled a passkey - the operator's authenticator (Touch ID,
Windows Hello, a roaming key) signs a challenge minted FOR THAT ONE COMMAND,
and the assertion is verified server-side before the command ever
dispatches.

Deliberately not `webauthn-rs` (that crate hard-depends on OpenSSL, a second
crypto backend this pure-Rust workspace does not want): a hand-parsed,
narrowly-scoped ceremony instead, ES256 only (`-7`, the one algorithm every
passkey provider ships) and attestation "none" (see
`crates/web/src/webauthn.rs`'s module doc for the full reasoning). Fail-closed
throughout: any parse or verify failure is a refusal, never a pass.

### Endpoints

- `GET /api/webauthn/passkeys` -> `{passkeys: [{credential_id, label,
  created_at}], webauthn_required: bool, policy_requires_passkey: bool}`. The
  one probe the frontend reads before rendering either the
  confirm-with-passkey flow or the software-signed badge;
  `webauthn_required` is simply "does this caller have at least one enrolled
  passkey", and `policy_requires_passkey` is "does this BOX refuse a
  sensitive command from a caller with none" (see the trial fallback below).
  The two are independent: the policy is the box's, the enrollment is the
  caller's.
- `POST /api/webauthn/register/start` `{operator_password?}` -> a
  `PublicKeyCredentialCreationOptions`-shaped JSON (`challenge` and
  `user.id` as base64url strings; the browser decodes them into
  `ArrayBuffer`s before calling `navigator.credentials.create`). Needs a
  factor the session does not carry, or no challenge is minted at all: the
  operator password for a FIRST enrollment (`403 {"webauthn":
  "password_required"}` without it), an `x-genaryx-webauthn` assertion bound
  to `webauthn_enroll_passkey` for every later one (`403 {"webauthn":
  "assertion_required"}` without it).
- `POST /api/webauthn/register/finish` `{label, credential_id,
  client_data_json, attestation_object}` (each base64url; `credential_id` is
  the base64url `rawId`) -> `{enrolled: true, credential_id}` on success.
- `POST /api/webauthn/passkeys/remove` `{credential_id, operator_password?}`
  -> `{removed: true, credential_id, remaining}`. An assertion bound to
  `webauthn_remove_passkey` and this exact `credential_id` removes any
  passkey while another remains; the LAST one takes the operator password and
  nothing else, because it is the removal that puts the box back to
  session-only. The password is accepted for a non-last removal too, which
  grants it no authority it did not have (remove the others one at a time,
  then the last) and ends the lockout an assertion-only rule would create
  when every enrolled key is lost at once. The "is this the last" check and
  the removal happen under one lock, so two concurrent removals cannot each
  see another key remaining and between them leave none.
- `POST /api/webauthn/action/start` `{command, args}` -> `{challenge, rp_id,
  timeout, user_verification, allow_credentials: [{type, id}]}`. Mints a
  challenge bound to the exact command name and a SHA-256 of the exact args
  JSON that dispatch will carry (`args_sha256`) - an assertion for "kill run
  A" can never be replayed to authorize "kill run B", or the same command
  with different arguments. `command` is one of the five sensitive commands,
  or one of the two lifecycle ceremony names above
  (`webauthn_enroll_passkey`, `webauthn_remove_passkey`), which are
  deliberately NOT in `SENSITIVE_COMMANDS`: they name an endpoint of this
  module, and a name that cannot be dispatched has no business in a list of
  commands.

### The header

Sensitive command dispatch (`POST /api/command/<name>`) carries the
assertion in an `x-genaryx-webauthn` request header: base64url of a JSON
envelope `{credential_id, client_data_json, authenticator_data, signature}`,
each field itself base64url of the raw bytes `navigator.credentials.get`
returned. Missing or malformed:

- No enrolled passkey at all: no header required, the command dispatches
  (the trial fallback below).
- A passkey is enrolled but no header was sent: `428 {"error": "a webauthn
  assertion is required for this command", "webauthn": "required"}`. The
  frontend (`lib/webauthn.ts`'s `invokeWithCeremony`) treats this exact shape
  as its retry signal: run the ceremony once, resend once.
- A header was sent but verification failed for any reason (a stale,
  replayed or foreign challenge, the wrong command or args binding, a bad
  signature, a cloned authenticator's regressed counter, ...): `403
  {"error": "webauthn: <reason>"}`.

### Relying-party configuration

`GENARYX_WEB_RP_ID` / `GENARYX_WEB_ORIGIN`, defaulting to the loopback
deployment: `rp_id = "localhost"`, `origin = "http://localhost:<bind
port>"`. A TLS-fronted console overrides both to its real domain; `rp_id`
must be the domain the browser addresses (WebAuthn scopes credentials to it)
and `origin` must be the exact `scheme://host[:port]` the browser reports in
`clientDataJSON.origin` - a mismatch on either is a refused ceremony, not a
silent pass.

### Secure-context deployment (the one real constraint)

`navigator.credentials` exists only in a secure context, so the ceremony
works when the console is reached as `localhost` (the default loopback bind,
or an `ssh -L` forward over the operator's tunnel) or behind TLS. A bare
`http://10.x.x.x` has no WebAuthn at all - not a degraded mode, an absent
one. The frontend's `webauthnAvailable()` (`lib/webauthn.ts`) is how the UI
knows this and says so honestly (`PasskeySettings`'s own hint line) instead
of showing an "Add passkey" button that would only fail.

### Trial fallback (no passkey enrolled), and how to switch it off

A caller with no enrolled passkey passes the gate without a ceremony -
`CommandRecord`'s own transport-signing fields stay exactly as they already
are on this shell, which read honestly as "software-signed". Nothing is
silently weakened: this is the documented, intentional bridge for an
operator who has not yet enrolled a passkey, not a bypass.
`PasskeySettings` explains exactly this to an operator with zero passkeys,
so enrolling the first one is framed as an upgrade, not a chore.

What was missing until 2026-08-05 was any way to STOP standing on the bridge:
an operator who wanted the ceremony unconditional had no setting to say so.
`GENARYX_WEB_REQUIRE_PASSKEY=1` (read once at startup, like the RP and OIDC
configuration beside it) makes it mandatory: a sensitive command from a
caller with nothing enrolled is refused with `403 {"webauthn":
"enrollment_required"}` and a message naming the command, the setting and
what to do about it. Deliberately NOT the `"required"` shape the frontend
retries on: re-running a ceremony that cannot exist yet would only fail
again. Off by default, so an upgrade changes nothing about a running box, and
`serve` states which way it read the variable at startup.

### The journal

A verified ceremony rides into the same `CommandRecord` the action already
journals: `sig_alg = "webauthn-es256"`, `sig_fpr = <credential id>` (the
exact enrolled credential that confirmed the action - an auditor can say
WHICH passkey, not just "a passkey"). The trial-fallback path leaves the
plane's own software-signing fields untouched, so the two stories are told
the same way a desktop Secure-Enclave kill and a software-signed one always
were.

### Frontend (`apps/web`)

`lib/webauthn.ts` is the one ceremony module: base64url helpers,
`webauthnAvailable()`, `listPasskeys()` (cached per page load,
`invalidatePasskeysCache()` after any change), `enrollPasskey(label,
operatorPassword?)`, `removePasskey(credentialId, operatorPassword?)`, and
`invokeWithCeremony(command, args)` - the wrapper `lib/money.ts`'s
`killRun`/`setBudget`, `lib/policy.ts`'s `decideApproval` and
`lib/remote.ts`'s `issueOperatorWgConfig`/`revokeOperatorWgPeer` call instead
of `invokeBackend` directly, so every existing caller of those five commands
inherits the ceremony with no panel-side change. `PasskeySettings.tsx`
(opened from the session area in `AppHeader.tsx`) lists enrolled passkeys,
adds and removes them, and shows the operator-password field exactly where
the server demands it (the first enrollment, the last removal); a plain
operator cancel of the platform's own passkey prompt is treated as "say
nothing", never an error banner.

Fixture-tested with a software authenticator
(`crates/web/src/webauthn.rs`'s `test_support`, `crates/web/src/main.rs`'s
gate tests, `apps/web/src/lib/webauthn.test.ts`); real hardware is the
live-pending step, the same posture D12's APNs push had before its own live
exit gate. Absorbs the "web-side signed kill" item from the genaryx README.
