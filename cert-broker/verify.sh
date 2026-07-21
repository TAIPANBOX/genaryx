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

echo "== ADVERSARIAL: assert the broker's gate DIRECTLY (specific codes) =="
# 403: valid creds, but a name that is not this relay's own.
code=$(curl -s -o /dev/null -w '%{http_code}' -u "$RELAY_USER:$RELAY_TOKEN" \
  -d "{\"fqdn\":\"_acme-challenge.victim-relay.$ZONE\",\"value\":\"x\"}" "$BROKER/present" || true)
[ "$code" = 403 ] && echo "   403 for a foreign name: subdomain gate held" \
  || { echo "   !! expected 403 for a foreign name, got $code"; exit 1; }
# 401: right relay id, wrong token.
code=$(curl -s -o /dev/null -w '%{http_code}' -u "$RELAY_USER:wrong-token" \
  -d "{\"fqdn\":\"_acme-challenge.$HOST\",\"value\":\"x\"}" "$BROKER/present" || true)
[ "$code" = 401 ] && echo "   401 for a bad token: auth held" \
  || { echo "   !! expected 401 for a bad token, got $code"; exit 1; }
# And end to end: the relay (lego) cannot complete an order for a foreign name.
rm -rf /tmp/cb-evil
if run_lego "victim-relay.$ZONE" /tmp/cb-evil >/dev/null 2>&1 && [ -d /tmp/cb-evil/certificates ]; then
  echo "   !! GATE FAILED: a cert was issued for a name the relay does not own"; exit 1
fi
echo "   end to end: no certificate for a foreign name"

echo "== cert broker verified end to end =="
