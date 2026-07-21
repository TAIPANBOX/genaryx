# Open work, carried out of the 2026-07-20 live campaign

This file exists because these items were decided in conversation and would
otherwise live only in a session that ends. Anything here is agreed work, not a
suggestion. Each entry says what was observed, why it matters, and what "done"
looks like, so it can be picked up cold.

Numbers in brackets are the task ids in the session task list, which does not
survive; this file is the durable copy.

---

## 1. stack-up must light up wave 1 AND wave 2, not just the daemons [#17]

**Asked for by Yurii, 2026-07-20:** one command brings up every wave-1 and
wave-2 service. Genaryx itself and the `taipan` deploy CLI are explicitly out of
scope: they are not started by that command.

**What it does today.** `up.sh` starts five real daemons and nothing else:
tokenfuse-gateway :4100, tokenfuse-cloud :8080, a static dashboard :3000,
wardryx :8090, idryx :8081.

**The nuance that changes the shape of the fix.** qryx, mockryx, engram and
verdryx are **not servers**. All four repos were read end to end during this
campaign: no serve subcommand, no port flag, no HTTP surface of any kind. qryx
scans a path and exits. mockryx sends crafted requests to a gateway you name and
exits. engram is a library plus a CLI plus a stdio-only MCP server over a local
SQLite file. verdryx is a CLI over a local SQLite file. Their own READMEs say so
("no server, no Docker"; "not a long-lived server the way Wardryx is").

So for those four, "bring it up" cannot mean "start a daemon". It can only mean:
put the binary where the console looks, and make its store exist.

Where the Genaryx console actually looks (crates/ffi/src/*/env.rs):

| plane   | binary                      | store                    |
|---------|-----------------------------|--------------------------|
| crypto  | `~/.taipan/bin/qryx`        | none, scans on demand    |
| drills  | `~/.taipan/bin/mockryx`     | none, runs on demand     |
| memory  | `~/.taipan/bin/engram-mcp`  | `~/.taipan/engram.engram`|
| quality | none, reads SQLite directly | `~/.taipan/verdryx.db`   |

**Unresolved design question, already sent to Fable:** stack-up installs into
`~/.stack-up/bin`, the console only knows `~/.taipan/`. Two homes, one of which
the console cannot see. Also unexplained: the `.marker-idryx`,
`.marker-tokenfuse`, `.marker-wardryx` files sitting in `~/.taipan/bin`. Work
out what they are for before changing the layout.

**Why this is not cosmetic.** On 2026-07-20 the Crypto, Drills, Memory and
Quality tabs rendered honest but empty states. They only produced content at all
because those files had survived on this Mac from earlier sessions. On a
customer's clean machine all four planes would be dead, which is precisely the
failure this whole "deployable by a customer, not by its author" push exists to
kill.

**Done looks like:** on a machine with an empty `~/.taipan`, one command leaves
all four discoverable, and Genaryx's Crypto, Drills, Memory and Quality tabs
render real content with no manual step. Seeding must be honest: an empty but
correct plane beats fake data dressed as real, so decide deliberately whether
seeding is default or flag-gated.

**DONE 2026-07-21**, stack-up commit `0d13a8f`. Both unresolved questions are
answered, and the answers are the load-bearing part:

- **The two homes.** Binaries go to `$TAIPAN_HOME/bin` (default
  `~/.taipan/bin`), the stack's published home; `~/.stack-up` keeps only logs,
  pids, clones, staging and stamps. This is forced, not chosen: `qryx` is
  looked up at exactly `~/.taipan/bin/qryx` with no env override and no PATH
  tier (`crates/ffi/src/crypto/env.rs`, a deliberate documented asymmetry), so
  an install anywhere else is invisible. Teaching the console a second
  discovery tier was rejected: shipped consoles cannot be patched
  retroactively, and the closed product must not track the open installer's
  layout. Symlinking was rejected because `cp` writes THROUGH a symlink, so a
  later `taipan up` would silently overwrite stack-up's own build output.
- **The `.marker-*` files** were never a mystery: `stale_paths <marker> <src
  roots>` rebuilds when any source is newer, and the marker is truncated after
  a successful build. `taipan` uses the identical convention and filenames in
  the same directory (`taipan/src/services/*.rs`), which is exactly why ours
  moved to `~/.stack-up/markers/` - sharing the directory would have each
  installer silently validating the other's build.
- **Two writers, so provenance not mtime**: every installed file's checksum is
  recorded; a file that does not match is another tool's and is left alone and
  reported (`--force-install` overrides). Installs are temp-file + rename,
  because `engram-mcp` may be running as the console's stdio child.
- **Seeding: no.** Stores are created with each tool's own code, only when
  absent, and empty. The money plane's demo dataset stays on because it lives
  in a process that forgets it on exit; these are files the console reads as
  ground truth, where demo rows would be indistinguishable from a customer's
  own forever. The summary prints the store paths and the first-real-row
  command instead.

Verified on a clean scratch home, plus each path separately (foreign binary
left intact then replaced under `--force-install`; a legacy `~/.stack-up/bin`
install moved rather than rebuilt; a venv missing an extra repaired). One real
bug fell out: engram's MCP server is an OPTIONAL extra, so a plain
`pip install .` produced an `engram-mcp` that exists, is executable, answers
`--help`, and dies on ImportError the first time anything speaks MCP to it. It
now installs `[mcp]` and verifies the import, and the check is part of the
up-to-date test so an already-broken venv repairs itself.

**Left deliberately undone:** `down.sh --purge` (an uninstall that removes only
files whose checksum matches our manifest, and never the stores) and recording
each tool's `--version` in the manifest so open-repo-HEAD vs console-contract
skew is diagnosable at a glance.

---

## 2. Provisioning must install and configure WireGuard on the box [#18]

**Asked for by Yurii, 2026-07-20:** if Genaryx connects to remote
infrastructure, WireGuard has to be installed and configured on that
infrastructure as part of provisioning it. Not afterwards, not by hand.

**What actually happened this campaign.** The fresh box had no wireguard package
at all, no `/etc/wireguard`, and ufw allowing only 22/tcp and 8443/tcp. Every one
of these steps was run by hand, mid-session:

```
apt-get install wireguard wireguard-tools
wg genkey / wg pubkey  ->  /root/wg/{server,client}.{key,pub}
write /etc/wireguard/wg0.conf   (10.99.0.1/24, ListenPort 51820)
ufw allow 51820/udp
systemctl enable --now wg-quick@wg0
wg set wg0 peer <console pubkey> allowed-ips 10.99.0.2/32
```

That last line is the worst of it: the console generates a keypair whose private
half never leaves Rust, so the operator has to read the public half off the
Remote panel and get it onto the box somehow. Today "somehow" was me, over SSH.

This is exactly the smell the standing rule names: manual scripts and
copy-pasted keys are a deployment-UX failure, and recovery paths count as much
as happy paths.

**What provisioning should do instead:** install the packages; generate the
server keypair 0600 and never print it; write `wg0.conf` with a documented
subnet and port; open that one UDP port and nothing else; publish the server
public key and endpoint through whatever the console already reads, so the
Remote panel arrives pre-filled instead of typed; and give the console's public
key a supported path onto the box that does not involve a human with an SSH
session.

**Also decide:** the peer subnet is currently invented per box. The box got
10.99.0.0/24 while the app defaulted to 10.9.0.x, and the two were reconciled by
hand. Pick one and make both sides default to it.

**Not in scope here:** the app's own banner correctly documents that
`wireguard-go` cannot create a tun device unprivileged, so Connect failing on an
operator's laptop is right, not a bug. This item is about the box side.

---

## 3. Remove the campaign box IP from the mobile project

`tokenfuse-mobile/ios/project.yml` carries an ATS exception naming the live
campaign box, `5.75.234.176`, on both the phone and the watch targets. It is
committed on branch `watch-relay-client-and-revocation` and marked in-file as
diagnostic rather than a shipping value, but it must not reach a release: it
names an ephemeral box, and it turns off ATS's own trust check for that host.

It exists because the relay serves a self-signed certificate that the app pins
from the QR, and ATS refuses the connection before the pinning delegate can
decide (observed: pin matched, then CFNetwork failed the task with -9802).
`NSAllowsArbitraryLoads` alone was not enough; the log showed
`enforce_ats(false) ... skip_ats_trust(false)` and the connection still failed.

This disappears on its own once the cert broker (item below, and task #14) gives
every relay a real trusted certificate. Until then, do not ship a build with it,
and re-check it whenever the campaign box changes.

## 4. Show the phone-pairing QR flow end to end

Owed to Yurii: a walkthrough of how the desktop console hands a phone its
pairing QR. The control lives on the **Pocket** tab (11th). It could not be
demonstrated on 2026-07-20 because both slots were already paired, so the panel
showed its paired state and a `Disconnect all` button instead of the QR.

Demonstrate: the unpaired empty state, the control that arms the QR, the QR
itself, the phone scanning it, and both devices arriving paired.

**Handle with care when capturing:** one QR carries the relay's pinned TLS
identity plus a live one-time code for each device, and the watch's code carries
kill authority for the life of the window. A QR must never appear un-redacted in
anything published.

## 5. Cert broker Step 4: real Let's Encrypt + Cloudflare [#14]

**Blocked on Yurii.** Steps 1-3 + the broker are DONE and scripted in
`genaryx/cert-broker/` (design A, README there). The relay's ACME client is
embedded (`crates/relay/src/acme.rs`); the broker has a pluggable DNS backend
(challtestsrv test + cloudflare prod), proven end to end on box `2.28.3.61`.

**Done looks like:** delegate `pocket.it-rat.com` to Cloudflare; mint a SCOPED
token (`Zone.DNS:Edit` on that zone ONLY) + its zone id; set
`BROKER_BACKEND=cloudflare` + the token in `/root/broker/broker.env`; on the
relay set `acme_directory_url`=Let's Encrypt + `acme_hostname` + `broker_*`. No
code change. Then the mobile `project.yml` ATS exception (`5.75.234.176`) goes
away (item 3), because the relay now serves a real trusted cert.

**Deferred (Fable review):** run the broker as a dedicated non-root user (not
root; relocate to `/opt`, PrivateTmp, RestrictAddressFamilies); and confirm the
phone->relay HTTP/2 hop carries the encoded path unchanged in the next live run.

## 6. Genaryx desktop: percent-encode ids in signed mutation paths [#21]

**DONE 2026-07-21**, genaryx commit `4ad8b83`. All three failure modes were
observed against a live tokenfuse-cloud by keeping the raw interpolation:
space + non-ASCII gave `403 signature_invalid` (the desync itself), a `#` made
`url` treat the rest of the path as a fragment so `/kill` fell off entirely
(404), and a `/` opened an extra segment and missed the route (404). New
`crates/connectors/src/urlpath.rs` encodes one id as one segment with the same
set Foundation uses on the phone, so console and phone produce identical paths;
empty/`.`/`..` fail closed (encoding does not help, the URL Standard treats
`%2e%2e` as a dot segment too), which also closes the mobile review's accepted
LOW. Swept the same class in `wardryx.rs`. Also closed the deferred "phone ->
relay hop" question without a live run: `crates/relay/src/proxy.rs` now proves
over a real HTTP round trip that the relay forwards an encoded path byte for
byte.

**The desktop twin of mobile #15.** `crates/connectors/src/cloud_rest.rs`
(~408/422/430) signs a mutation over a canonical path built by interpolating a
run/agent id RAW, then reqwest percent-encodes it AFTER signing -> signature
desync for any id with a reserved char (space, `#`, `?`, non-ASCII); the Cloud
verifies over `uri.path()`, the raw encoded path. Ids are customer-controlled,
so this is realistic. Fable-confirmed with url 2.5.8.

**Done looks like:** encode each dynamic segment as a single path segment
(`utf8_percent_encode`, encoding `/`), build ONE path, sign THAT, send exactly
those bytes; mirror the mobile fix (`asPathSegment`/`Account.mutationURL` in
tokenfuse-mobile, commits `eac44ed` + `ed9c9de`). Add a test.
