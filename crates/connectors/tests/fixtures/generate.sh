#!/usr/bin/env bash
# Regenerate the TLS fixtures uapi_tls_pinning.rs needs.
#
# The two .crt files are committed; the leaf's private key is not, because the
# repository's .gitignore keeps `*.key` out of the tree and a public repo is
# the last place to make an exception. Without the key that test skips, so run
# this once if you want it to actually run.
#
# It is a PAIR on purpose, and that is the lesson the test exists for: a CA
# that signs one leaf, the client pins the CA, the proxy serves the leaf.
# rustls refuses a single self-signed certificate used as both
# (`CaUsedAsEndEntity`), because `openssl req -x509` marks it `CA:TRUE`.
set -euo pipefail
cd "$(dirname "$0")"

openssl req -x509 -newkey rsa:2048 -nodes -days 3650 \
  -keyout ca.key -out ca.crt -subj "/CN=uapi-test-ca" >/dev/null 2>&1

openssl req -newkey rsa:2048 -nodes -keyout uapi-proxy.key -out proxy.csr \
  -subj "/CN=wg-uapi-proxy" >/dev/null 2>&1

# The SAN carries the real Service name AND localhost, so the test can dial a
# loopback listener while still verifying a NAME. No IP entry: one of the two
# tests depends on dialling by address being refused.
openssl x509 -req -in proxy.csr -CA ca.crt -CAkey ca.key -CAcreateserial \
  -out uapi-proxy.crt -days 3650 \
  -extfile <(printf 'subjectAltName=DNS:wg-uapi-proxy,DNS:localhost\n') >/dev/null 2>&1

# The CA key has done its one job. Keeping it would create an authority with a
# lifecycle; rotating means running this script again.
rm -f ca.key proxy.csr ca.srl
echo "wrote ca.crt, uapi-proxy.crt, uapi-proxy.key"
