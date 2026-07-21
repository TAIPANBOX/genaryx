#!/usr/bin/env bash
# Authorize one console (or a stand-in) as a WireGuard peer on this box.
#
#   add-peer.sh <console-public-key> [tunnel-ip]
#
# The console generates its own keypair whose PRIVATE half never leaves the
# app; you pass its PUBLIC half here. Default tunnel IP is 10.9.0.2, the peer
# address the console's Remote panel defaults to (crates/ffi/src/remote/env.rs).
#
# This is the supported path today. OPEN-WORK #18 wants to remove the human in
# the middle (the operator reading the pubkey off the panel and running this),
# so treat this as the interim mechanism, not the finished deployment UX.
set -euo pipefail

PUBKEY="${1:?usage: add-peer.sh <console-public-key> [tunnel-ip]}"
PEER_IP="${2:-10.9.0.2}"
WG_IF="wg0"

wg set "$WG_IF" peer "$PUBKEY" allowed-ips "${PEER_IP}/32"
echo "authorized peer ${PUBKEY:0:8}... at ${PEER_IP}/32 on ${WG_IF}"
