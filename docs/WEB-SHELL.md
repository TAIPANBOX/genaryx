# Running the console on the customer's box

`genaryx-web` is the browser shell of the console: the only shell this
product ships. It runs **inside the customer's perimeter**, beside their
stack, and answers every request by calling `genaryx-api`, the command layer
this shell wraps. Nothing about their runs, spend or identities travels
anywhere to be displayed, and it-rat.com has no route to it.

## Build

```sh
cd apps/web && pnpm build     # browser bundle -> apps/web/dist
cd ../..    && cargo build -p genaryx-web --release
```

## First run

One operator account per box. The password is read from stdin so it never
lands in the process list, and it is stored as an Argon2id hash, never in a
form anything can reverse.

```sh
echo 'a-long-passphrase' | ./target/release/genaryx-web \
  set-password --username ops
```

```sh
./target/release/genaryx-web serve \
  --bind 127.0.0.1:7420 \
  --ui apps/web/dist
```

### Where to bind

The default is loopback, and that is a security decision rather than a
convenience one. The console is reached over the operator's own WireGuard
tunnel (D11), so the address worth binding is the tunnel's:

```sh
--bind 10.9.0.1:7420
```

Binding a wildcard address works but is logged as a warning at startup. Add
`--secure-cookies` only when something in front terminates TLS: over plain
HTTP inside the tunnel a `Secure` cookie is simply never sent back, which
locks the operator out with no explanation.

## IdP login and roles

One local operator account (above) always works. If the customer runs an
identity provider, `genaryx-web` can also verify its OIDC ID-tokens, and it
does so entirely offline: no `.well-known` discovery, no outbound call to the
IdP, ever. It is off unless configured, and configuring it never removes the
local account.

Set all three of these before `serve` starts, or OIDC stays off and the
console shows only the local login:

```sh
export GENARYX_WEB_OIDC_ISSUER=https://login.microsoftonline.com/<tenant-id>/v2.0
export GENARYX_WEB_OIDC_AUDIENCE=genaryx-console
export GENARYX_WEB_OIDC_JWKS=/etc/genaryx/jwks.json   # inline JSON also works
```

Four more are optional, each defaulting to what a plain OIDC token already
carries:

- `GENARYX_WEB_OIDC_SUB_CLAIM` (default `sub`): the claim used as the
  signed-in username.
- `GENARYX_WEB_OIDC_ROLES_CLAIM` (default `roles`): the claim read for role
  mapping.
- `GENARYX_WEB_OIDC_ADMIN_ROLE` (default `genaryx-admin`): the roles-claim
  value that grants the `admin` role.
- `GENARYX_WEB_OIDC_APPROVER_ROLE` (default `genaryx-approver`): the
  roles-claim value that grants the `approver` role.

`GENARYX_WEB_OIDC_JWKS` is read once at startup, inline JSON or a file path,
and never fetched again: no live call to the IdP, air-gap safe by
construction, the same contract tokenfuse-cloud's own OIDC already uses.
Rotating the IdP's signing key means updating the file (or the env var) and
restarting `genaryx-web`; there is no background refresh to fall back on.
`GET /api/auth/session` reports `oidc_available: true` once the three
required vars are set, and the browser then shows "Sign in with your
organization" next to the local login form.

The local Argon2id account (`set-password`, above) is unchanged by any of
this: it stays the break-glass path onto the box, and it is always `admin`,
IdP configured or not.

Every session, local or OIDC, carries one of three roles, and `genaryx-web`
checks it before a command runs, not after:

- `viewer`: reads. Every status and list endpoint, the live bus, onboard
  status, crypto scan and verify, Felyx ask/explain/status.
- `approver`: viewer, plus granting or denying a Wardryx approval.
- `admin`: approver, plus every privileged mutation: kill a run, set a
  budget, ack an incident, forget a memory, remote WireGuard/SSH, identity
  rescan, drills, onboard generate/write, pair or unpair Pocket, the copilot
  proposal-approval log, and evidence build.

An OIDC session's role comes from the roles claim: `admin` if it contains the
admin role, `approver` if it contains the approver role, otherwise `viewer`;
it cannot end up with more access than the token's own claim grants. This is
also why the audit trail now names the signed-in person
(`user://<org>/<sub>` for OIDC, `user://<org>/<username>` for local) instead
of the box's OS account.

Roles gate the command surface, not the moment of action: the right role plus
a live session is what a role check can prove, and nothing in it re-signs an
action as it happens. That is what the per-action WebAuthn ceremony is for,
and it has landed (part 2, below). The full contract and the honest limits
are in docs/CONSOLE-IDP.md.

## The per-action passkey ceremony

Five commands carry it (`crates/web/src/main.rs`'s `SENSITIVE_COMMANDS`):
`money_kill_run`, `money_set_budget`, `policy_decide_approval`,
`remote_operator_wg_config` and `remote_operator_wg_revoke`. Each needs a
fresh assertion from the operator's own passkey, bound to that exact command
and its exact arguments, verified before the command dispatches.

Enrolling and removing a passkey are part of the same control, and neither
rides on the session:

- the FIRST passkey is enrolled with the operator password
  (`set-password`, above); every later one with an assertion from a passkey
  already enrolled;
- a passkey is removed with an assertion from an enrolled one, and the LAST
  one only with the operator password, since that is the removal that takes
  the console back to session-only. It is also the recovery path when the
  only authenticator is lost: the password removes it, and the password
  enrols its replacement.

```sh
# Refuse a sensitive command outright when the caller has no passkey,
# instead of running it on the session and journaling it software-signed.
export GENARYX_WEB_REQUIRE_PASSKEY=1
```

Off by default, so an upgrade changes nothing on a running box. With it off,
a caller with no enrolled passkey still runs those five commands and the
journal records them honestly as software-signed: a weaker state, deliberately
kept as the bridge for an operator who has not enrolled yet. With it on, that
bridge is gone and the refusal says to enrol a passkey. `genaryx-web serve`
states which way it read the variable at startup.

The browser needs a secure context for any of this: reach the console as
`localhost` (the loopback bind, or an `ssh -L` forward over the tunnel) or
behind TLS. A bare `http://10.x.x.x` has no WebAuthn at all, and the panel
says so rather than showing controls that could only fail.

## Pointing it at the stack

Every plane resolves its own environment from a `taipan up` descriptor under
`$TAIPAN_HOME/environments/` (default `~/.taipan/environments/`). If the stack
was started by `taipan up`, this already exists and there is nothing to do.

A stack brought up by hand has no descriptor, and each plane then reports a
clean "no environment" state rather than guessing. Writing one by hand is
enough, and the shape is small. `<name>.json`:

```json
{
  "name": "live",
  "events": { "dir": "/root/.stack-up/events" },
  "services": {
    "cloud":   { "url": "http://127.0.0.1:8080" },
    "gateway": { "url": "http://127.0.0.1:4100" },
    "idryx":   { "url": "http://127.0.0.1:8081" },
    "wardryx": { "url": "http://127.0.0.1:8090" },
    "verdryx": { "url": "/root/.taipan/verdryx.db" }
  },
  "keys": {
    "cloud_admin_ref":   "taipan/live/cloud_admin",
    "wardryx_admin_ref": "taipan/live/wardryx_admin"
  }
}
```

Bearer tokens live in a sibling `<name>.keys.json`, never in the descriptor
itself, and only the trailing segment of a `*_ref` is looked up in it:

```json
{ "secrets": { "cloud_admin": "...", "wardryx_admin": "..." } }
```

`chmod 600` that file. Notes worth having in advance, each learned by getting
it wrong:

- `services.verdryx.url` is a **filesystem path** to `verdryx.db`, not a URL:
  Verdryx has no serve process to pair with.
- The Money plane keys off `cloud`, Drills off `gateway`. They are not
  interchangeable names for the same service.
- Policy needs `keys.wardryx_admin_ref` as well as the service URL. With the
  URL alone it stays at "no environment", even though Wardryx is healthy.
- `events.dir` is what puts the Bus Explorer into live mode. Without it the
  console runs a synthetic demo feeder and says so, rather than showing
  invented numbers as if they were real.
- Newest descriptor wins when several exist, by modification time.

## When a panel is empty, ask why

```sh
./target/release/genaryx-web doctor
```

It reports every plane and, for anything unresolved, names the actual gap:
the missing `services.<key>`, the missing `keys.<ref>`, the secret a ref
points at that is not in the keyfile, or the url field that is really a path.
It exits non-zero, so it can gate a deploy, and the same findings are logged
at startup so nobody has to know the subcommand exists.

Read `ok` carefully: `resolved from live` means it read your descriptor, while
`NOT reading your descriptor: it fell back to ...` means something else is
carrying that plane. The second still works today and fails the day the
fallback goes away, which is why it is reported as a problem rather than an
"ok".

The `remote` line is reported but never failed. Unlike every other plane, the
Remote panel has no discoverable environment by design: the peer, the SSH
target and the binary path are things the operator sets per campaign, so a
missing default is not a fault.

## Verifying it is actually live

```sh
curl -s -c j -X POST localhost:7420/api/auth/login \
  -H 'content-type: application/json' \
  -d '{"username":"ops","password":"..."}'

curl -s -b j -X POST localhost:7420/api/command/money_status \
  -H 'content-type: application/json' -d '{}'
```

`"state":"ready"` with `"source":{"source":"taipan"}` means it resolved the
descriptor. `"source":"env_fallback"` means it did not and is using
`TOKENFUSE_CLOUD_ADMIN_KEY` against `127.0.0.1:8080` instead.

For the live bus, open the stream and make something happen on it:

```sh
curl -s -N -b j --max-time 25 localhost:7420/api/events &
curl -s -b j -X POST localhost:7420/api/command/drills_run \
  -H 'content-type: application/json' \
  -d '{"scenario_dir":"/path/to/scenarios","api_key":null,
       "fail_on_skip":false,"save_path":null}'
```

A quiet stack producing no SSE frames is correct, not a fault: the stream
carries what the bus receives, and a stack nobody is calling receives nothing.

## What the API looks like

- `POST /api/auth/login` / `logout`, `GET /api/auth/session`
- `POST /api/command/<name>`: the body is the args
  object the frontend already sends, a 2xx body is the command's Ok value, and
  a **422 body is the command's own Err value unwrapped**, so each plane's
  existing error normaliser works untouched. 400 is malformed arguments, 401 is
  no session, 404 is an unknown command. For the five sensitive commands, 428
  means "send an assertion" and 403 means the ceremony was refused.
- `GET /api/webauthn/passkeys`, `POST /api/webauthn/passkeys/remove`,
  `POST /api/webauthn/register/start` / `register/finish`,
  `POST /api/webauthn/action/start`: the passkey lifecycle and the ceremony.
- `GET /api/events` is the live bus as Server-Sent Events (`event: bus`).

Signing in opens the console, at whatever role the session carries. It does
not by itself run a destructive action: those five re-sign in the moment they
happen, with the operator's passkey, whenever one is enrolled (always, with
`GENARYX_WEB_REQUIRE_PASSKEY=1`). docs/CONSOLE-IDP.md has the full contract
and the honest limits.
