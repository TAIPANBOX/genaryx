#!/usr/bin/env bash
set -uo pipefail
export DEBIAN_FRONTEND=noninteractive
echo "[$(date +%H:%M:%S)] apt update+base..."
apt-get update -qq && apt-get install -y -qq build-essential git curl pkg-config libssl-dev python3-venv python3-pip python3-dev unzip ca-certificates jq >/dev/null 2>&1 && echo "  apt OK"
echo "[$(date +%H:%M:%S)] rust (rustup minimal)..."
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal >/dev/null 2>&1 && echo "  rust OK: $($HOME/.cargo/bin/rustc --version 2>/dev/null)"
echo "[$(date +%H:%M:%S)] go 1.26.5..."
curl -sSL https://go.dev/dl/go1.26.5.linux-amd64.tar.gz -o /tmp/go.tgz && rm -rf /usr/local/go && tar -C /usr/local -xzf /tmp/go.tgz && echo "  go OK: $(/usr/local/go/bin/go version)"
grep -q cargo/bin /root/.bashrc || echo 'export PATH=$PATH:/usr/local/go/bin:$HOME/.cargo/bin:$HOME/go/bin; export GOTOOLCHAIN=auto' >> /root/.bashrc
echo "[$(date +%H:%M:%S)] TOOLCHAINS DONE"
