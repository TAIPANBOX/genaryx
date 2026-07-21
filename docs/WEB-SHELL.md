# Running the console on the customer's box

`genaryx-web` is the browser shell of the same console the desktop app ships.
It runs **inside the customer's perimeter**, beside their stack, and answers
every request by calling `genaryx-api`, the identical functions the Tauri shell
wraps. Nothing about their runs, spend or identities travels anywhere to be
displayed, and it-rat.com has no route to it.

## Build

```sh
cd apps/desktop && npm run build:web     # browser bundle -> apps/desktop/dist-web
cd ../..         && cargo build -p genaryx-web --release
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
  --ui apps/desktop/dist-web
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
- `POST /api/command/<name>` mirrors Tauri's `invoke`: the body is the args
  object the frontend already sends, a 2xx body is the command's Ok value, and
  a **422 body is the command's own Err value unwrapped**, so each plane's
  existing error normaliser works untouched. 400 is malformed arguments, 401 is
  no session, 404 is an unknown command.
- `GET /api/events` is the live bus as Server-Sent Events (`event: bus`).

Signing in opens the console. It does not authorise a destructive action:
those are re-signed at the moment they happen, so a stolen session can look
but not act.
