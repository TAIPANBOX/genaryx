#!/usr/bin/env bash
export PATH="$PATH:/usr/local/go/bin:/root/.cargo/bin:/root/go/bin"
export GOTOOLCHAIN=auto
pkill -9 -f tokenfuse 2>/dev/null; pkill -9 -f "bin/wardryx" 2>/dev/null; pkill -9 -f "idryx serve" 2>/dev/null
sleep 3
cd /root/stack-up || exit 1
exec ./up.sh --no-demo
