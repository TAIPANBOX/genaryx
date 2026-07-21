#!/bin/zsh
# The Phase 3 stand: Cloud (money + incidents) <- relay (bounded read) <- phone.
# Cloud state is in-memory, so a restart means re-seeding; this is the whole
# sequence in one place so a later run reproduces the same surface.
set -e
SP=/private/tmp/claude-501/-Users-factory/9c636509-e8f3-4343-bf84-a694e949e33f/scratchpad

TOKENFUSE_CLOUD_KEYS="devkey:default:admin:paid,relaykey:default:viewer:paid" \
PORT=8083 \
nohup ~/Development/tokenfuse/target/debug/tokenfuse-cloud > $SP/cloud.log 2>&1 &
echo "cloud pid $!"
sleep 2

GENARYX_RELAY_ORG=default \
GENARYX_RELAY_CLOUD_BASE_URL=http://127.0.0.1:8083 \
GENARYX_RELAY_CLOUD_VIEWER_KEY=relaykey \
GENARYX_RELAY_PUBLIC_BIND_ADDR=127.0.0.1:8443 \
GENARYX_RELAY_ADMIN_BIND_ADDR=127.0.0.1:8444 \
GENARYX_RELAY_PUBLIC_ADVERTISE_URL=https://127.0.0.1:8443 \
GENARYX_RELAY_TLS_CERT_DIR=$SP/relay-tls \
GENARYX_RELAY_DB_PATH=$SP/relay-db/relay.sqlite \
nohup ~/Development/genaryx/target/debug/genaryx-relay > $SP/relay.log 2>&1 &
echo "relay pid $!"
sleep 3
