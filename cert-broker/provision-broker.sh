#!/usr/bin/env bash
# Install the Pocket cert broker as a systemd service. Works for BOTH backends:
# challtestsrv (test) and cloudflare (production). Idempotent. Run as root on
# the box, from this directory.
#
# For production (Step 4): edit /root/broker/broker.env to set
# BROKER_BACKEND=cloudflare and the scoped CLOUDFLARE_API_TOKEN + CLOUDFLARE_ZONE_ID
# before starting; the token then lives ONLY in that 0600 file on this box.
set -euo pipefail

DIR="$(cd "$(dirname "$0")" && pwd)"
BROKER_DIR="${BROKER_DIR:-/root/broker}"
mkdir -p "$BROKER_DIR"

echo "== 1. broker code =="
cp "$DIR/broker.py" "$BROKER_DIR/broker.py"

echo "== 2. environment (first run installs the example; never overwrites yours) =="
if [ ! -f "$BROKER_DIR/broker.env" ]; then
  cp "$DIR/broker.env.example" "$BROKER_DIR/broker.env"
  chmod 600 "$BROKER_DIR/broker.env"
  echo "   wrote $BROKER_DIR/broker.env (edit it before going to cloudflare)"
else
  echo "   $BROKER_DIR/broker.env kept"
fi

echo "== 3. relay tokens (first run seeds the proof relay; never overwrites yours) =="
if [ ! -f "$BROKER_DIR/relays.json" ]; then
  echo '{"proof01":"proof-relay-token"}' > "$BROKER_DIR/relays.json"
  chmod 600 "$BROKER_DIR/relays.json"
  echo "   wrote $BROKER_DIR/relays.json (one proof relay). Add real relays here."
else
  echo "   $BROKER_DIR/relays.json kept"
fi

echo "== 4. systemd unit =="
cp "$DIR/systemd/pocket-broker.service" /etc/systemd/system/
systemctl daemon-reload
systemctl reset-failed pocket-broker 2>/dev/null || true
systemctl enable --now pocket-broker >/dev/null 2>&1
systemctl restart pocket-broker
sleep 2
echo "   pocket-broker: $(systemctl is-active pocket-broker)"
# 401 without auth == up and gated.
code=$(curl -s -o /dev/null -w '%{http_code}' -X POST http://127.0.0.1:9000/present -d '{}' || true)
echo "   broker /present without auth -> HTTP $code (401 = up, auth-gated)"

echo "== broker installed. Test path: ./verify.sh =="
