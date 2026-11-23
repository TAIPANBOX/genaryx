#!/usr/bin/env bash
# The console image runs as the real console, on the architecture that built
# it (CLAUDE.md invariant 52). Run by .github/workflows/release.yml on a pull
# request and on a tag, against the image it just built:
#
#   scripts/image-smoke.sh <image> <version, e.g. v0.1.0>
#
# What it holds, each a way a correct-looking image has been wrong:
#  1. Every bundled tool EXECUTES here. A binary built for another
#     architecture does not exit non-zero, the kernel refuses it with
#     "exec format error", so that is what is looked for, plus no output at
#     all; exit codes are ignored (qryx answers --help with 1).
#  2. verdryx can read a Parquet trace (pyarrow is in the image).
#  3. `genaryx-web --version` is the version being released.
#  4. The image runs as 10001:10001, numerically.
#  5. The server answers /healthz, and / is the real-box bundle: its assets
#     are at /assets/, where the demo build's --base=./ writes ./assets/.
set -euo pipefail

img="${1:?usage: image-smoke.sh <image> <version>}"
want="${2:?usage: image-smoke.sh <image> <version>}"
want="${want#v}"
fail=0

echo "== on $(uname -m), image $img"
docker run --rm --entrypoint /bin/sh "$img" -c '
  fail=0
  for t in qryx mockryx verdryx wardryx "tokenfuse constants" genaryx-web; do
    out=$(command -v "${t%% *}" >/dev/null 2>&1 && $t 2>&1 | head -5 || echo "__NOT_ON_PATH__")
    case "$out" in
      *"exec format error"*) echo "  $t: EXEC FORMAT ERROR, built for another architecture"; fail=1 ;;
      __NOT_ON_PATH__)       echo "  $t: not on PATH in this image"; fail=1 ;;
      "")                    echo "  $t: ran and said nothing at all"; fail=1 ;;
      *)                     echo "  $t: runs" ;;
    esac
  done
  command -v engram-mcp >/dev/null 2>&1 && echo "  engram-mcp: on PATH" || { echo "  engram-mcp: not on PATH"; fail=1; }
  if python3 -c "import pyarrow.parquet" 2>/dev/null; then
    echo "  verdryx can read a Parquet trace"
  else
    echo "  verdryx slo cannot read a trace: pyarrow is not in this image"; fail=1
  fi
  exit $fail
' || fail=1

got=$(docker run --rm --entrypoint /usr/local/bin/genaryx-web "$img" --version 2>&1 || true)
if [ "$got" = "genaryx-web $want" ]; then
  echo "  version: $got"
else
  echo "  version: the binary says '$got', the release is $want"; fail=1
fi

user=$(docker inspect --format '{{.Config.User}}' "$img")
if [ "$user" = "10001:10001" ]; then
  echo "  user: $user"
else
  echo "  user: '$user', expected 10001:10001"; fail=1
fi

name="genaryx-smoke-$$"
docker run -d --name "$name" "$img" >/dev/null
trap 'docker rm -f "$name" >/dev/null 2>&1 || true' EXIT
probe='
import sys, time, urllib.request
for _ in range(60):
    try:
        h = urllib.request.urlopen("http://127.0.0.1:7420/healthz", timeout=2).status
        page = urllib.request.urlopen("http://127.0.0.1:7420/", timeout=2).read().decode("utf-8", "replace")
        break
    except Exception:
        time.sleep(1)
else:
    print("  server: no answer on 127.0.0.1:7420 within 60 s"); sys.exit(1)
if h != 200:
    print(f"  server: /healthz answered {h}"); sys.exit(1)
if "/assets/" not in page or "./assets/" in page:
    print("  ui: / is not the real-box bundle (assets not at /assets/)"); sys.exit(1)
print("  server: /healthz 200, / is the real-box bundle")
'
if ! docker exec "$name" python3 -c "$probe"; then
  docker logs "$name" 2>&1 | tail -20 | sed 's/^/  log: /'
  fail=1
fi

if [ "$fail" -ne 0 ]; then
  echo "FAIL: the image does not run as the console it claims to be"
  exit 1
fi
echo "OK: every tool executes, version $want, user 10001:10001, server and real-box UI answer"
