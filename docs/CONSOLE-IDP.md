# Console IdP, roles, and named audit actors (D15/B3, part 1)

Status: built on branch `feat/console-idp-roles`. Design record and the exact
contract. Part 1 of B3 (itrat-console/15); the WebAuthn per-action ceremony
is part 2, a separate branch.

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
- Roles gate the console's command surface; they are NOT a second signature.
  A destructive action is still only a session away in part 1. Part 2
  (WebAuthn) is what re-signs each destructive action with a hardware-backed
  passkey; until it lands, the box's exposure is still "session opens the
  console" (docs/WEB-SHELL.md), now narrowed by role.

## Part 2 (next branch): WebAuthn per-action ceremony

Passkey registration in an authenticated session; a per-action assertion
required for kill / break-glass / budget / policy write / approval grant;
server-side verification; the assertion (alg + credential id) recorded in the
CommandRecord, giving the browser the same hardware-attested story the
desktop Touch ID and the phone Face ID already have. Fixture-tested with a
software authenticator; real hardware is the live-pending step (like APNs in
D12). Absorbs the "web-side signed kill" item from the genaryx README.
