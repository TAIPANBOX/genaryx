#!/usr/bin/env bash
set -uo pipefail
export PATH=$PATH:/usr/local/go/bin:/root/.cargo/bin:/root/go/bin
export GOTOOLCHAIN=auto
GH=https://github.com/TAIPANBOX
log(){ echo "[$(date +%H:%M:%S)] $*"; }
mkdir -p /root/bin
log "waiting for toolchains..."
for i in $(seq 1 240); do grep -q "TOOLCHAINS DONE" /root/setup.log 2>/dev/null && break; sleep 5; done
grep -q "TOOLCHAINS DONE" /root/setup.log || { log "ABORT: toolchains not ready"; exit 1; }
log "toolchains ready: $(/usr/local/go/bin/go version | awk '{print $3}'), $(/root/.cargo/bin/rustc --version | awk '{print $2}')"
cd /root
# stack-up (money/policy/identity) - detached, keeps serving
git clone --depth 1 $GH/stack-up.git >/dev/null 2>&1 && log "cloned stack-up"
( cd /root/stack-up && setsid nohup ./up.sh > /root/stack.log 2>&1 < /dev/null & echo $! > /root/stack.pid )
log "stack-up launched pid $(cat /root/stack.pid 2>/dev/null) -> /root/stack.log"
# qryx (crypto)
git clone --depth 1 $GH/qryx.git >/dev/null 2>&1 && log "cloned qryx, building (go1.27 auto-fetch)..."
( cd /root/qryx && go build -o /root/bin/qryx ./cmd/qryx 2>>/root/build.err && log "qryx BUILT" ) || log "qryx build FAILED (see build.err)"
# mockryx (drills)
git clone --depth 1 $GH/mockryx.git >/dev/null 2>&1 && log "cloned mockryx, building..."
( cd /root/mockryx && go build -o /root/bin/mockryx ./cmd/mockryx 2>>/root/build.err && log "mockryx BUILT" ) || log "mockryx build FAILED"
# engram (memory, python) + verdryx (quality, python)
git clone --depth 1 $GH/engram.git >/dev/null 2>&1 && log "cloned engram, venv+install (fastembed is heavy)..."
( cd /root/engram && python3 -m venv .venv && ./.venv/bin/pip install -q -U pip >/dev/null 2>&1 && ./.venv/bin/pip install -q -e '.[mcp]' 2>>/root/build.err && log "engram INSTALLED: $(ls .venv/bin/engram-mcp 2>/dev/null && echo has-mcp)" ) || log "engram install FAILED"
git clone --depth 1 $GH/verdryx.git >/dev/null 2>&1 && log "cloned verdryx, venv+install..."
( cd /root/verdryx && python3 -m venv .venv && ./.venv/bin/pip install -q -e . 2>>/root/build.err && log "verdryx INSTALLED" ) || log "verdryx install FAILED"
log "DEPLOY PIPELINE DONE"
