#!/usr/bin/env bash
export PATH="$PATH:/usr/local/go/bin:/root/.cargo/bin:/root/go/bin"
pkill -f "idryx serve --addr 127.0.0.1:8082" 2>/dev/null
sleep 1
exec /root/.stack-up/bin/idryx serve --addr 127.0.0.1:8082 --load tokenfuse:/tmp/meridian-idryx.ndjson
