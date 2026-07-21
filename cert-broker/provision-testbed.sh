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
install_release() {  # $1 = asset-name-substring  $2 = dest
  local sub="$1" dest="$2"
  [ -x "$dest" ] && { echo "   $dest present"; return; }
  local url
  url=$(curl -sSL https://api.github.com/repos/letsencrypt/pebble/releases/latest \
        | grep -oE '"browser_download_url": *"[^"]*"' | cut -d'"' -f4 \
        | grep -E "$sub" | grep -E 'linux-amd64|linux_amd64' | head -1)
  [ -n "$url" ] || { echo "   !! no release asset matching $sub"; exit 1; }
  echo "   downloading $(basename "$url")"
  curl -sSL "$url" -o "$dest"
  chmod +x "$dest"
}
install_release 'pebble-challtestsrv|pebble_challtestsrv' /usr/local/bin/pebble-challtestsrv
install_release 'pebble[-_]linux|/pebble-linux|pebble-v' /usr/local/bin/pebble
# Fallback: some release layouts name the main binary just "pebble-linux-amd64".
[ -x /usr/local/bin/pebble ] || install_release 'pebble' /usr/local/bin/pebble

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
    "listenAddress": "0.0.0.0:14000",
    "managementListenAddress": "0.0.0.0:15000",
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
systemctl enable --now challtestsrv pebble >/dev/null 2>&1
sleep 3
echo "   challtestsrv: $(systemctl is-active challtestsrv)   pebble: $(systemctl is-active pebble)"
curl -sk -m5 https://127.0.0.1:14000/dir >/dev/null && echo "   Pebble ACME directory answers" || { echo "   !! Pebble not answering"; exit 1; }

echo "== testbed up. Now run ./provision-broker.sh, then ./verify.sh =="
