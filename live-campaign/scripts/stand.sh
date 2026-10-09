#!/bin/zsh
# The Phase 3 stand: Cloud (money + incidents) <- relay (bounded read) <- phone.
# Cloud state is in-memory, so a restart means re-seeding; this is the whole
# sequence in one place so a later run reproduces the same surface.
set -e
SP=/private/tmp/claude-501/-Users-factory/9c636509-e8f3-4343-bf84-a694e949e33f/scratchpad

# The cloud's two keys, minted fresh for this run: the admin key (seeding,
# gx_trickle_misbehave.py) and the relay's viewer key. They used to be the
# literals `devkey` and `relaykey`; tokenfuse#380 removed the cloud's devkey
# fallback, and a key written into a public repository is no key either way.
# An exported TOKENFUSE_CLOUD_ADMIN_KEY is used as the admin key instead, the
# same name the gx_*.py scripts read it from. The spec is key:org:role: the old
# fourth segment `:paid` was a plan once and is parsed as a SITE by today's
# tokenfuse (key:org[:role[:site]]), so it is gone.
CLOUD_KEY="${TOKENFUSE_CLOUD_ADMIN_KEY:-$(openssl rand -hex 16)}"
RELAY_KEY="$(openssl rand -hex 16)"
if [[ -z "$CLOUD_KEY" || -z "$RELAY_KEY" ]]; then
  echo "could not mint the cloud keys (openssl rand)" >&2; exit 1
fi
if [[ "$CLOUD_KEY" == *[:,[:space:]]* ]]; then
  echo "TOKENFUSE_CLOUD_ADMIN_KEY must not contain ':', ',' or whitespace (it goes into TOKENFUSE_CLOUD_KEYS)" >&2; exit 2
fi

TOKENFUSE_CLOUD_KEYS="$CLOUD_KEY:default:admin,$RELAY_KEY:default:viewer" \
PORT=8083 \
nohup ~/Development/tokenfuse/target/debug/tokenfuse-cloud > $SP/cloud.log 2>&1 &
echo "cloud pid $!"
echo "cloud admin key: $CLOUD_KEY"
echo "  for gx_trickle_misbehave.py: export TOKENFUSE_CLOUD_ADMIN_KEY=$CLOUD_KEY"
sleep 2

GENARYX_RELAY_ORG=default \
GENARYX_RELAY_CLOUD_BASE_URL=http://127.0.0.1:8083 \
GENARYX_RELAY_CLOUD_VIEWER_KEY="$RELAY_KEY" \
GENARYX_RELAY_PUBLIC_BIND_ADDR=127.0.0.1:8443 \
GENARYX_RELAY_ADMIN_BIND_ADDR=127.0.0.1:8444 \
GENARYX_RELAY_PUBLIC_ADVERTISE_URL=https://127.0.0.1:8443 \
GENARYX_RELAY_TLS_CERT_DIR=$SP/relay-tls \
GENARYX_RELAY_DB_PATH=$SP/relay-db/relay.sqlite \
nohup ~/Development/genaryx/target/debug/genaryx-relay > $SP/relay.log 2>&1 &
echo "relay pid $!"
sleep 3
