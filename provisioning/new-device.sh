#!/usr/bin/env bash
# Issue a ready-to-import WireGuard config for one more device.
#
#   new-device.sh <name> [tunnel-ip]
#
# Prints a complete `.conf` and, when `qrencode` is present, the same config as
# a QR code in the terminal. Point a phone's camera at the QR, or import the
# file into the desktop WireGuard app: either way there is nothing to type.
#
# WHY THIS EXISTS ALONGSIDE add-peer.sh
#
# `add-peer.sh` takes a public key you already have, because the desktop
# console generates its own keypair and its private half never leaves the app.
# That is the right shape for the console and the wrong shape for everything
# else: a phone or a laptop running the OFFICIAL WireGuard client has no way to
# hand you a public key without a human copying it out of a UI, which is the
# step this is meant to remove. So here the box generates the pair, authorizes
# the public half, and hands over a finished config. That is also exactly what
# it-rat.com already describes ("your box issues a WireGuard config as a QR
# code, and it is scanned by the official WireGuard app").
#
# THE PRIVATE KEY IS IN THE OUTPUT. Treat it like one:
#   - it is written to stdout and never to disk by this script;
#   - `wg genkey` output is not logged;
#   - if you redirect it to a file, chmod 600 that file and delete it once the
#     device has imported it.
#
# It also solves the chicken and egg the console cannot: to open the console you
# must already be on the tunnel, and to be on the tunnel you need a config from
# the console. The first device is issued here, over SSH; every device after
# that can be issued from the console itself.
set -euo pipefail

NAME="${1:?usage: new-device.sh <name> [tunnel-ip]}"
PEER_IP="${2:-}"
WG_IF="wg0"
WG_PORT="51820"
SUBNET="10.9.0.0/24"

command -v wg >/dev/null || { echo "wg not found: run provision-wireguard.sh first" >&2; exit 1; }
wg show "$WG_IF" >/dev/null 2>&1 || { echo "interface $WG_IF is not up: run provision-wireguard.sh first" >&2; exit 1; }

# Next free address in the tunnel subnet, so two devices never collide. 10.9.0.1
# is the box itself; .2 is the desktop console's default (see add-peer.sh), so
# anything issued here starts at .3 unless told otherwise.
if [ -z "$PEER_IP" ]; then
  taken="$(wg show "$WG_IF" allowed-ips | grep -oE '10\.9\.0\.[0-9]+' || true)"
  for n in $(seq 3 254); do
    if ! grep -qx "10.9.0.$n" <<<"$taken"; then PEER_IP="10.9.0.$n"; break; fi
  done
  [ -n "$PEER_IP" ] || { echo "no free address left in $SUBNET" >&2; exit 1; }
fi

SERVER_PUB="$(wg show "$WG_IF" public-key)"
# Fail rather than hand over a config that cannot connect: an empty server key
# produces a file the WireGuard app imports happily and then never completes a
# handshake with, which is a miserable thing to debug on someone else's laptop.
[ -n "$SERVER_PUB" ] || { echo "could not read the server public key from $WG_IF" >&2; exit 1; }
ENDPOINT_HOST="$(curl -fsS --max-time 5 https://api.ipify.org 2>/dev/null || true)"
if [ -z "$ENDPOINT_HOST" ]; then
  ENDPOINT_HOST="$(ip -4 route get 1.1.1.1 2>/dev/null | awk '{print $7; exit}')"
fi
[ -n "$ENDPOINT_HOST" ] || { echo "could not determine this box's public address; pass it in by hand" >&2; exit 1; }

PRIV="$(wg genkey)"
PUB="$(printf '%s' "$PRIV" | wg pubkey)"

# Authorize before printing: if this fails, the operator must not walk away
# with a config that was never going to connect.
wg set "$WG_IF" peer "$PUB" allowed-ips "${PEER_IP}/32"
wg-quick save "$WG_IF" 2>/dev/null || true

CONF="$(cat <<EOF
# genaryx tunnel: ${NAME}
[Interface]
PrivateKey = ${PRIV}
Address    = ${PEER_IP}/32

[Peer]
PublicKey  = ${SERVER_PUB}
Endpoint   = ${ENDPOINT_HOST}:${WG_PORT}
# Only the tunnel subnet, deliberately. 0.0.0.0/0 would pull every packet on
# the device through this box, which is not what a console tunnel is for and
# reads as "your product broke my internet" the first time someone tries it.
AllowedIPs = ${SUBNET}
PersistentKeepalive = 25
EOF
)"

echo "authorized ${NAME} as ${PUB:0:8}... at ${PEER_IP}/32 on ${WG_IF}" >&2
echo >&2

printf '%s\n' "$CONF"

if command -v qrencode >/dev/null; then
  echo >&2
  echo "scan this with the official WireGuard app:" >&2
  printf '%s\n' "$CONF" | qrencode -t ansiutf8 >&2
else
  echo >&2
  echo "(install qrencode for a scannable QR: apt-get install -y qrencode)" >&2
fi
