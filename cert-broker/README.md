# Pocket cert broker (design A)

Gives every `genaryx-relay` a **real, publicly-trusted TLS certificate** for its
own `<relay-id>.pocket.it-rat.com`, so the Pocket phone/watch connect to it by
hostname with ordinary system trust: no SPKI pin, no ATS exception. This is the
IT-RAT-side infrastructure for task #14; the relay-side ACME client lives in
`crates/relay/src/acme.rs`.

## The two invariants (why this shape)

1. **The DNS-zone credential lives ONLY on the broker.** The relay never gets a
   Cloudflare token. It asks the broker to publish the `_acme-challenge` TXT.
2. **The certificate private key lives ONLY on the relay.** The relay runs the
   ACME order itself and generates its own key; the broker only ever sees the
   challenge token, never a CSR or a key.

The broker also enforces that a relay may publish a challenge **only for its own
subdomain** (`_acme-challenge.<relay-id>.pocket.it-rat.com`), authenticated by a
per-relay token. Relay A cannot obtain relay B's certificate.

```
relay (acme.rs)                     broker (broker.py)              DNS zone
  order <id>.pocket.it-rat.com
  POST /present {fqdn, value}  --->  auth relay, check fqdn  --->   TXT set
  CA validates, signs
  key stays on the relay
  POST /cleanup {fqdn, value}  --->                          --->   TXT removed
```

## Layout

| file | what |
|------|------|
| `broker.py` | the broker: `/present` + `/cleanup`, per-relay auth, subdomain gate, pluggable DNS backend |
| `broker.env.example` | environment template (backend, zone, tokens) |
| `systemd/pocket-broker.service` | the broker service |
| `systemd/pebble.service`, `systemd/challtestsrv.service` | the local test ACME server + DNS mock |
| `provision-broker.sh` | install the broker (both backends) |
| `provision-testbed.sh` | install the local Pebble + DNS-mock proof stack |
| `verify.sh` | prove the flow end to end (happy path + adversarial) |

Everything is scripted because the box gets rebuilt: nothing here depends on
state left on a machine by hand.

## Backends

`BROKER_BACKEND` selects where the challenge TXT is published:

- **`challtestsrv`** - the local `pebble-challtestsrv` mock. Proof/test only.
- **`cloudflare`** - the real `pocket.it-rat.com` zone via a **scoped** token
  (`Zone.DNS:Edit` on that one zone), which lives only in the broker's `0600`
  env file and is never logged.

## Run the proof (test path, no real DNS or CA)

On a fresh box, as root, from this directory:

```
./provision-testbed.sh    # Pebble (fake Let's Encrypt) + challtestsrv (DNS mock)
./provision-broker.sh     # the broker, BROKER_BACKEND=challtestsrv by default
./verify.sh               # lego (standing in for the relay) issues a real cert,
                          # then proves the subdomain gate holds
```

To prove it with the relay's OWN code instead of lego, run the ignored
integration test from the Mac over an SSH tunnel to the box's loopback:

```
ssh -f -N -L 127.0.0.1:14000:127.0.0.1:14000 -L 127.0.0.1:9000:127.0.0.1:9000 <box>
scp <box>:/root/broker/ca-cert.pem /tmp/pebble-ca.pem
RELAY_ACME_DIR=https://127.0.0.1:14000/dir RELAY_ACME_BROKER=http://127.0.0.1:9000 \
RELAY_ACME_BROKER_USER=proof01 RELAY_ACME_BROKER_TOKEN=proof-relay-token \
RELAY_ACME_HOST=proof01.pocket.it-rat.com RELAY_ACME_CA=/tmp/pebble-ca.pem \
  cargo test -p genaryx-relay acme::tests::obtains_a_real_certificate -- --ignored --nocapture
```

## Step 4: go live (needs the zone + a token)

1. Delegate `pocket.it-rat.com` to Cloudflare (so the broker's token can never
   touch the `it-rat.com` site records).
2. Mint a **scoped** Cloudflare token: `Zone.DNS:Edit` on `pocket.it-rat.com`
   only. Note its `CLOUDFLARE_ZONE_ID`.
3. In `/root/broker/broker.env`: `BROKER_BACKEND=cloudflare`, and set
   `CLOUDFLARE_API_TOKEN` + `CLOUDFLARE_ZONE_ID`. `systemctl restart pocket-broker`.
4. On the relay, point `acme_directory_url` at Let's Encrypt
   (`https://acme-v02.api.letsencrypt.org/directory`), set `acme_hostname` +
   `broker_*`. The relay now gets a browser-trusted cert; Pebble/challtestsrv are
   not involved at all.

Nothing in `broker.py` or the relay changes between test and production: the
directory URL is a relay-side flag, the DNS backend is this one env var.

Once live, the temporary `project.yml` ATS exception in `tokenfuse-mobile` is
removed (it only existed because the relay served a self-signed cert).

## Security notes

- Keep the broker on loopback (or a private path the relay shares); do not
  expose it to the internet. `ufw` should allow only what the box actually needs.
- `broker.env` (Cloudflare token) and `relays.json` (relay tokens) are `0600`.
- The token is scoped to one zone. Even if the box is compromised, it cannot
  touch `it-rat.com` or any other zone.
