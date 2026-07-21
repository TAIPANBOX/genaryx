#!/usr/bin/env bash
# Installs the pre-push hook that runs `check-all.sh --fast`.
#
#   bash scripts/install-hooks.sh          install
#   bash scripts/install-hooks.sh --remove uninstall
#
# Opt-in on purpose. A hook that gates every push is a real change to how
# this repo feels to work in, so it is never installed as a side effect of
# anything else. The full gate stays a deliberate `bash scripts/check-all.sh`
# (and, once GitHub Actions is unblocked, CI running the same script).
#
# What the hook buys: `--fast` compiles the workspace, the Tauri shell's Rust
# and the frontend's types. That is exactly the class of breakage that went
# unnoticed for four days in July 2026 (see check-all.sh's header). It does
# not run the test suites or build the SwiftUI xcframework, so it stays in
# the tens of seconds rather than the minutes.
#
# Bypass for a deliberate WIP push: `git push --no-verify`.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
HOOK="$ROOT/.git/hooks/pre-push"

if [ "${1:-}" = "--remove" ]; then
  if [ -e "$HOOK" ]; then
    rm "$HOOK"
    echo "removed $HOOK"
  else
    echo "nothing to remove: $HOOK does not exist"
  fi
  exit 0
fi

if [ -e "$HOOK" ]; then
  echo "refusing to overwrite an existing hook: $HOOK" >&2
  echo "inspect it, then re-run with --remove first if you want ours." >&2
  exit 1
fi

cat > "$HOOK" <<'EOF'
#!/usr/bin/env bash
# Installed by scripts/install-hooks.sh. Remove with:
#   bash scripts/install-hooks.sh --remove
# Bypass once with: git push --no-verify
exec bash "$(git rev-parse --show-toplevel)/scripts/check-all.sh" --fast
EOF

chmod +x "$HOOK"
echo "installed $HOOK"
echo "it runs: bash scripts/check-all.sh --fast"
echo "bypass once with: git push --no-verify"
