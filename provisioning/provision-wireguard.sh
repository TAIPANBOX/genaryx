#!/usr/bin/env bash
# Provision WireGuard on a box so the Genaryx console can reach it over a
# tunnel, and NOTHING else can. This is the box side of OPEN-WORK #18: run it
# once during provisioning, not by hand mid-session.
#
# It installs the packages, generates the server keypair (0600, never printed),
# writes wg0.conf on the subnet/port the console defaults to (10.9.0.1/24 :51820,
# reconciling the earlier hand-picked 10.99.x), opens exactly that one UDP port
# plus SSH, and prints ONLY the server public key and endpoint, which is what
# the console's Remote panel needs to fill itself in. The private key never
# leaves the box; the console's public key is added as a peer separately (a
# human reading it off the panel is what #18 wants to remove, but the add path
# below is the supported one until then).
set -euo pipefail

WG_ADDR="10.9.0.1/24"        # server tunnel IP; console peer is 10.9.0.2
WG_PORT="51820"
WG_IF="wg0"
CONF="/etc/wireguard/${WG_IF}.conf"

export DEBIAN_FRONTEND=noninteractive
command -v wg >/dev/null 2>&1 || apt-get install -y -qq wireguard wireguard-tools

umask 077
mkdir -p /etc/wireguard
if [ ! -s /etc/wireguard/server.key ]; then
  wg genkey > /etc/wireguard/server.key
  wg pubkey < /etc/wireguard/server.key > /etc/wireguard/server.pub
  chmod 600 /etc/wireguard/server.key
fi

if [ ! -s "$CONF" ]; then
  cat > "$CONF" <<EOF
[Interface]
Address = ${WG_ADDR}
ListenPort = ${WG_PORT}
PrivateKey = $(cat /etc/wireguard/server.key)
# Peers (the console, and any stand-in) are added at runtime with:
#   wg set ${WG_IF} peer <pubkey> allowed-ips 10.9.0.2/32
EOF
  chmod 600 "$CONF"
fi

# Firewall: the whole point. Only SSH and the one WG port face the internet.
# The stack's services bind 127.0.0.1, so they are already off the internet;
# they are reached over the tunnel via a forward on the WG interface (set up
# separately). ufw is the belt to that braces.
if command -v ufw >/dev/null 2>&1; then
  ufw --force reset >/dev/null 2>&1 || true
  ufw default deny incoming >/dev/null
  ufw default allow outgoing >/dev/null
  ufw allow 22/tcp >/dev/null
  ufw allow ${WG_PORT}/udp >/dev/null
  # Allow the tunnel's INNER traffic. Without this, ufw's default-deny drops
  # decrypted packets arriving on wg0 (a peer pinging 10.9.0.1, or reaching a
  # service forwarded onto the WG interface), so the handshake succeeds but
  # nothing flows. This permits only already-authenticated peers, never the
  # internet, which still sees just SSH and the one UDP port.
  ufw allow in on ${WG_IF} >/dev/null
  ufw --force enable >/dev/null
fi

systemctl enable --now "wg-quick@${WG_IF}" >/dev/null 2>&1 || {
  # Bring it up directly if systemd unit path differs.
  wg-quick up "$WG_IF" >/dev/null 2>&1 || true
}

echo "WG_SERVER_PUBKEY=$(cat /etc/wireguard/server.pub)"
echo "WG_ENDPOINT=$(curl -fsS -4 ifconfig.me 2>/dev/null || hostname -I | awk '{print $1}'):${WG_PORT}"
echo "WG_INTERFACE_IP=10.9.0.1"
echo "WG_PROVISIONED_OK"
