#!/usr/bin/env bash
# Enforces invariant 1 of CLAUDE.md: the console never stores cloud credentials.
#
# Multi-cloud inventory is read-only and runs through the operator's OWN CLI,
# already authenticated on their machine. The console never holds a key, so
# there is nothing on it worth stealing for somebody else's cloud account.
#
# That is a product decision, not an implementation detail, and it is the kind
# that erodes for a good reason: an operator without the CLI installed, or a
# hosted deployment where spawning a binary is awkward, and suddenly reading
# AWS_SECRET_ACCESS_KEY looks pragmatic. From that point the console is a target
# and its whole proposition has changed.
#
# Two things are checked, and both are structural:
#
#   1. No cloud credential environment variable is read anywhere. Reading one is
#      how a key gets into the process; once it is in, "never stores" becomes a
#      promise about handling rather than about possession.
#   2. No cloud provider SDK is declared. An SDK exists to authenticate, and
#      pulling one in is the same decision arriving under another name. The
#      operator's CLI is spawned instead, which is the whole design.
#
# This file is the ONE copy of this check.

set -uo pipefail
cd "$(git rev-parse --show-toplevel)" || exit 1

problems=0

# The provider list is the point of failure here, not the grep. Until
# 2026-08-06 it covered AWS, GCP, Azure, IBM and OpenStack and no Hetzner term
# at all, while the console shipped a Hetzner inventory connector and the
# estate runs its own Kubernetes baseline on Hetzner. A gate that names every
# provider except the one actually in use reads as enforcement and is not.
# Verified by breaking it: a `std::env::var("HCLOUD_TOKEN")` in `crates/`
# passed this script cleanly before HCLOUD_TOKEN was on this line.
CRED='AWS_ACCESS_KEY_ID|AWS_SECRET_ACCESS_KEY|AWS_SESSION_TOKEN|GOOGLE_APPLICATION_CREDENTIALS|GOOGLE_CREDENTIALS|GCP_SERVICE_ACCOUNT|AZURE_CLIENT_SECRET|AZURE_TENANT_ID|ARM_CLIENT_SECRET|IBM_CLOUD_API_KEY|OS_PASSWORD|HCLOUD_TOKEN|HCLOUD_API_TOKEN|HETZNER_TOKEN|HETZNER_API_TOKEN|DIGITALOCEAN_TOKEN|DO_API_TOKEN'

while IFS= read -r hit; do
	[ -n "$hit" ] || continue
	echo "FAIL: $hit"
	echo "      Reading a cloud credential is how a key gets into this process."
	echo "      The inventory runs through the operator's own CLI, already"
	echo "      authenticated on their machine, precisely so the console holds none."
	problems=$((problems + 1))
done < <(grep -rnE "$CRED" --include='*.rs' --include='*.ts' --include='*.tsx' crates/ apps/ 2>/dev/null)

SDK='^(aws-sdk-|aws-config|rusoto|google-cloud|gcp_auth|azure_identity|azure_core|azure_storage|object_store)'
for manifest in $(git ls-files 'Cargo.toml' '*/Cargo.toml'); do
	while IFS= read -r dep; do
		[ -n "$dep" ] || continue
		echo "FAIL: $manifest declares $dep"
		echo "      An SDK exists to authenticate. Pulling one in is the same"
		echo "      decision arriving under another name; the design spawns the"
		echo "      operator's CLI instead."
		problems=$((problems + 1))
	done < <(grep -oE "$SDK[a-z0-9_-]*" "$manifest" 2>/dev/null)
done

if [ "$problems" -ne 0 ]; then
	echo
	echo "A console that holds cloud keys is a target, and it is a different"
	echo "product. See CLAUDE.md invariant 1."
	exit 1
fi

echo "OK: no cloud credential is read and no provider SDK is declared."
