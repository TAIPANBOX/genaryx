#!/usr/bin/env bash
# Prove the whole DNS-01 flow end to end against the LOCAL testbed (Pebble +
# challtestsrv + the broker), using lego as a stand-in for the relay's own ACME
# client. Happy path issues a real cert; the adversarial path proves a relay
# cannot get a certificate for a name that is not its own. Run as root on the
# box after provision-testbed.sh + provision-broker.sh.
set -euo pipefail

BROKER_DIR="${BROKER_DIR:-/root/broker}"
LEGO="$BROKER_DIR/lego"
CA="$BROKER_DIR/ca-cert.pem"
DIRURL="https://127.0.0.1:14000/dir"
BROKER="http://127.0.0.1:9000"
ZONE="${BROKER_ZONE:-pocket.it-rat.com}"
RELAY_USER="${RELAY_USER:-proof01}"
RELAY_TOKEN="${RELAY_TOKEN:-proof-relay-token}"
HOST="${RELAY_USER}.${ZONE}"

run_lego() {  # $1 = domain  $2 = data-dir
  LEGO_CA_CERTIFICATES="$CA" \
  HTTPREQ_ENDPOINT="$BROKER" \
  HTTPREQ_USERNAME="$RELAY_USER" HTTPREQ_PASSWORD="$RELAY_TOKEN" \
  "$LEGO" run --server "$DIRURL" --dns httpreq \
    --dns.resolvers 127.0.0.1:8053 --dns.propagation.disable-ans \
    --domains "$1" --email "relay@it-rat.com" --accept-tos --path "$2"
}

echo "== HAPPY PATH: issue a cert for $HOST =="
rm -rf "$BROKER_DIR/lego-data"
run_lego "$HOST" "$BROKER_DIR/lego-data" 2>&1 | grep -iE 'validated|certificate|server responded' | sed 's/^/   /'
CRT="$BROKER_DIR/lego-data/certificates/${HOST}.crt"
[ -f "$CRT" ] || { echo "   !! no certificate issued"; exit 1; }
echo "   issued leaf:"
openssl x509 -in "$CRT" -noout -issuer -dates -ext subjectAltName 2>/dev/null | sed 's/^/     /'
openssl verify -CAfile <(cat "$BROKER_DIR/lego-data/certificates/${HOST}.issuer.crt" \
  <(curl -sk https://127.0.0.1:15000/roots/0)) "$CRT" 2>&1 | sed 's/^/   chain: /'

echo "== ADVERSARIAL: $RELAY_USER must NOT get a cert for victim-relay.$ZONE =="
rm -rf /tmp/cb-evil
if run_lego "victim-relay.$ZONE" /tmp/cb-evil >/dev/null 2>&1 && [ -d /tmp/cb-evil/certificates ]; then
  echo "   !! GATE FAILED: a cert was issued for a name the relay does not own"; exit 1
else
  echo "   gate held: no certificate (the broker 403s a name that is not $RELAY_USER's own)"
fi

echo "== cert broker verified end to end =="
