#!/usr/bin/env bash
# Stand up the LOCAL proof/test bed for the cert broker: a Pebble ACME server
# (a stand-in for Let's Encrypt) plus pebble-challtestsrv (a DNS-01 mock), as
# systemd units bound so only loopback reaches them. Idempotent: safe to re-run.
#
# This is the test path only. Production issuance uses real Let's Encrypt +
# the Cloudflare backend and does NOT need Pebble/challtestsrv (see README,
# "Step 4"). Run as root on the box, from this directory.
set -euo pipefail

DIR="$(cd "$(dirname "$0")" && pwd)"
BROKER_DIR="${BROKER_DIR:-/root/broker}"
mkdir -p "$BROKER_DIR"

echo "== 1. binaries (pebble, pebble-challtestsrv) =="
# Pebble releases ship the binary inside a .tar.gz as
# <tool>-linux-amd64/linux/amd64/<tool>, NOT as a raw asset, so download and
# EXTRACT (a plain `curl -o` leaves a gzip file marked +x -> "Exec format
# error"). Pinned rather than `releases/latest` so a future asset-layout change
# cannot silently break a fresh box.
PEBBLE_VER="${PEBBLE_VER:-v2.10.1}"
install_pebble() {  # $1 = binary name (pebble | pebble-challtestsrv)  $2 = dest
  local tool="$1" dest="$2"
  [ -x "$dest" ] && { echo "   $dest present"; return; }
  local url="https://github.com/letsencrypt/pebble/releases/download/${PEBBLE_VER}/${tool}-linux-amd64.tar.gz"
  local tmp; tmp=$(mktemp -d)
  echo "   downloading ${tool} ${PEBBLE_VER}"
  curl -fsSL "$url" | tar xz -C "$tmp" \
    || { echo "   !! download/extract failed: $url"; rm -rf "$tmp"; exit 1; }
  local bin; bin=$(find "$tmp" -type f -name "$tool" | head -1)
  [ -n "$bin" ] || { echo "   !! ${tool} binary not found in the archive"; rm -rf "$tmp"; exit 1; }
  install -m 0755 "$bin" "$dest"
  rm -rf "$tmp"
}
install_pebble pebble-challtestsrv /usr/local/bin/pebble-challtestsrv
install_pebble pebble /usr/local/bin/pebble

echo "== 2. lego (the relay's ACME-client stand-in for the proof) =="
if [ ! -x "$BROKER_DIR/lego" ]; then
  VER=$(curl -sSL https://api.github.com/repos/go-acme/lego/releases/latest | grep -oP '"tag_name": *"\K[^"]+')
  echo "   lego $VER"
  curl -sSL "https://github.com/go-acme/lego/releases/download/${VER}/lego_${VER}_linux_amd64.tar.gz" \
    | tar xz -C "$BROKER_DIR" lego
fi
"$BROKER_DIR/lego" --version | head -1 | sed 's/^/   /'

echo "== 3. a local test CA that signs Pebble's endpoint cert =="
# macOS (and rustls-platform-verifier) reject server leaf certs valid for more
# than ~398 days, so the LEAF gets 397 days; the CA can be long-lived.
if [ ! -f "$BROKER_DIR/ca-cert.pem" ]; then
  openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes \
    -keyout "$BROKER_DIR/ca-key.pem" -out "$BROKER_DIR/ca-cert.pem" -days 3650 \
    -subj "/CN=Pocket Pebble Test CA" \
    -addext "basicConstraints=critical,CA:TRUE" \
    -addext "keyUsage=critical,keyCertSign,cRLSign" 2>/dev/null
  echo "   CA created"
fi
if [ ! -f "$BROKER_DIR/pebble-cert.pem" ]; then
  openssl req -newkey ec -pkeyopt ec_paramgen_curve:P-256 -nodes \
    -keyout "$BROKER_DIR/pebble-key.pem" -out "$BROKER_DIR/pebble.csr" \
    -subj "/CN=pebble" 2>/dev/null
  cat > "$BROKER_DIR/ext.cnf" <<EOF
subjectAltName=DNS:localhost,IP:127.0.0.1
extendedKeyUsage=serverAuth
basicConstraints=critical,CA:FALSE
EOF
  openssl x509 -req -in "$BROKER_DIR/pebble.csr" \
    -CA "$BROKER_DIR/ca-cert.pem" -CAkey "$BROKER_DIR/ca-key.pem" -CAcreateserial \
    -out "$BROKER_DIR/pebble-cert.pem" -days 397 -extfile "$BROKER_DIR/ext.cnf" 2>/dev/null
  echo "   Pebble endpoint cert signed (397d, SAN localhost/127.0.0.1, serverAuth)"
fi

echo "== 4. pebble config =="
cat > "$BROKER_DIR/pebble-config.json" <<EOF
{
  "pebble": {
    "listenAddress": "127.0.0.1:14000",
    "managementListenAddress": "127.0.0.1:15000",
    "certificate": "$BROKER_DIR/pebble-cert.pem",
    "privateKey": "$BROKER_DIR/pebble-key.pem",
    "httpPort": 5002,
    "tlsPort": 5001,
    "ocspResponderURL": "",
    "externalAccountBindingRequired": false
  }
}
EOF

echo "== 5. systemd units =="
cp "$DIR/systemd/challtestsrv.service" "$DIR/systemd/pebble.service" /etc/systemd/system/
systemctl daemon-reload
systemctl reset-failed challtestsrv pebble 2>/dev/null || true
systemctl enable challtestsrv pebble >/dev/null 2>&1
systemctl restart challtestsrv pebble   # restart so a re-run applies unit/config changes
sleep 3
echo "   challtestsrv: $(systemctl is-active challtestsrv)   pebble: $(systemctl is-active pebble)"
curl -sk -m5 https://127.0.0.1:14000/dir >/dev/null && echo "   Pebble ACME directory answers" || { echo "   !! Pebble not answering"; exit 1; }

echo "== testbed up. Now run ./provision-broker.sh, then ./verify.sh =="
