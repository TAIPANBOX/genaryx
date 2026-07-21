# Box-side provisioning (D11 remote channel)

The Genaryx desktop console reaches a remote box over a **WireGuard tunnel**,
not over the public internet: the stack's services bind `127.0.0.1`, the box
exposes only SSH and one WireGuard UDP port, and the console talks to the
services through the tunnel. This directory provisions the box side of that.

This is distinct from [`../cert-broker`](../cert-broker), which gives the
**relay** a publicly-trusted certificate so the **phone/watch** trust it by
hostname. That is a mobile concern; the desktop console needs none of it.

## Scripts

| script | what |
|--------|------|
| `provision-wireguard.sh` | install WireGuard, generate the server keypair (0600, never printed), write `wg0.conf` on `10.9.0.1/24 :51820` (the subnet/port the console defaults to, `crates/ffi/src/remote/env.rs`), lock the firewall to SSH + the one WG port + already-authenticated tunnel traffic, bring the interface up. Prints only the server public key and endpoint. |
| `new-device.sh <name> [ip]` | issue a ready-to-import config for one more device: generate the peer keypair ON THE BOX, authorize the public half, pick the next free tunnel address, and print a complete `.conf` plus a scannable QR. For phones and laptops running the OFFICIAL WireGuard client, which cannot hand you a public key without a human copying it out of a UI. Also the way the FIRST device gets on: the console cannot issue that one, since reaching the console needs the tunnel it would be issuing. `AllowedIPs` is the tunnel subnet only, never `0.0.0.0/0`. The private key is printed, never written to disk. |
| `add-peer.sh` | authorize one console's public key as a peer at `10.9.0.2/32`. |

## Flow

```sh
# on the box, once, during provisioning:
sudo bash provision-wireguard.sh
#   -> WG_SERVER_PUBKEY=...   (put these two into the console's Remote panel)
#   -> WG_ENDPOINT=<ip>:51820

# authorize the console (its Remote panel shows its own public key):
sudo bash add-peer.sh <console-public-key>
```

The console then Connects, and the stack answers over the tunnel and nowhere
else.

## Verified

Proven end to end on a real box (2026-07-21): server up, a peer connected,
the cloud reachable over the tunnel at `10.9.0.1:8080` while the same service
timed out from the public internet. The one non-obvious rule that fix
depends on is `ufw allow in on wg0`: without it the handshake completes but
ufw's default-deny drops the decrypted inner traffic, so nothing flows.

## What OPEN-WORK #18 still wants

This is the box-side core, not the finished deployment UX. Still open:

- **No human relaying keys.** Today the operator reads the console's public
  key off the panel and runs `add-peer.sh`; #18 wants a supported path that
  removes that step.
- **Publish the server pubkey + endpoint through what the console already
  reads**, so the Remote panel arrives pre-filled instead of typed.
- **Service exposure on the tunnel** is currently a separate forward from the
  loopback-bound services onto the WG interface; decide whether the stack
  should bind the WG address directly instead.
