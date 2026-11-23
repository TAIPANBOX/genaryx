#!/usr/bin/env bash
# The release workflow publishes only from a tag, and a pull request can build
# the image but never push, sign or release it (CLAUDE.md invariant 52).
#
# WHY THIS EXISTS
#
# .github/workflows/release.yml runs on pull requests on purpose, so the image
# build is exercised before its first tag. That makes one edit dangerous: a
# job holding the package scope, the OIDC token or `contents: write` whose
# `if:` is dropped or widened would publish, sign or release from a pull
# request. Nothing else would notice; the run would be green.
#
# WHAT IT HOLDS
#
#  1. The workflow's top-level permissions are `contents: read` and nothing
#     else, and no workflow here triggers on `pull_request_target`.
#  2. Every job that holds `packages: write`, `id-token: write`,
#     `attestations: write` or `contents: write` has an `if:` that names a
#     push or a workflow_dispatch event and names no pull request.
#  3. A job that runs on a pull request holds nothing beyond
#     `contents: read`, logs in to no registry, and pushes nothing.
#  4. Every action is pinned by a full 40-character commit SHA.
#  5. The Dockerfile builds the real-box bundle (`--mode web`) and never names
#     the demo build (`build:demo`, `--mode mock`).
#
# It refuses to report OK when the workflow or the Dockerfile is not here, or
# when the workflow has no job.
set -uo pipefail
cd "$(git rev-parse --show-toplevel)" || exit 1

wf=".github/workflows/release.yml"
df="Dockerfile"
for f in "$wf" "$df"; do
	[ -f "$f" ] || { echo "measured nothing: $f is not here"; exit 2; }
done

fail=0

if grep -nE '^[[:space:]]*pull_request_target' .github/workflows/*.yml >/dev/null; then
	echo "FAIL: a workflow triggers on pull_request_target: $(grep -lE '^[[:space:]]*pull_request_target' .github/workflows/*.yml | tr '\n' ' ')"
	fail=1
fi

ruby -ryaml -e '
  wf = YAML.safe_load(File.read(ARGV[0]), aliases: false)
  jobs = wf.is_a?(Hash) ? wf["jobs"] : nil
  if !jobs.is_a?(Hash) || jobs.empty?
    puts "measured nothing: #{ARGV[0]} has no jobs"
    exit 2
  end
  bad = []
  top = wf["permissions"]
  bad << "top-level permissions are #{top.inspect}, not {contents: read}" unless top == { "contents" => "read" }
  jobs.each do |name, job|
    perms = job["permissions"].is_a?(Hash) ? job["permissions"] : {}
    cond = job["if"].to_s
    strong = perms.select { |k, v| v == "write" }.keys
    on_pr = cond.include?("pull_request")
    if !strong.empty?
      names_tag_event = cond =~ /event_name\s*==\s*.(push|workflow_dispatch)./
      if cond.strip.empty? || !names_tag_event || on_pr || cond.include?("!=")
        bad << "job #{name} holds #{strong.join(", ")} but its if: (#{cond.strip.empty? ? "none" : cond.strip}) does not restrict it to a push or a dispatch"
      end
    end
    if on_pr
      bad << "job #{name} runs on a pull request and holds #{strong.join(", ")}" unless strong.empty?
      (job["steps"] || []).each do |st|
        uses = st["uses"].to_s
        with = st["with"].is_a?(Hash) ? st["with"] : {}
        bad << "job #{name} runs on a pull request and logs in to a registry" if uses.start_with?("docker/login-action")
        bad << "job #{name} runs on a pull request and pushes (#{uses})" if with["push"] == true || with["push"].to_s == "true" || with["outputs"].to_s.include?("push=true")
      end
    end
    (job["steps"] || []).each do |st|
      uses = st["uses"].to_s
      next if uses.empty? || uses.start_with?("./")
      bad << "job #{name}: #{uses} is not pinned by a full commit SHA" unless uses =~ /@[0-9a-f]{40}\z/
    end
  end
  bad.each { |b| puts "FAIL: #{b}" }
  exit(bad.empty? ? 0 : 1)
' "$wf"
rc=$?
[ "$rc" -eq 2 ] && exit 2
[ "$rc" -eq 0 ] || fail=1

# Instructions only: a comment may say why the demo build is not here.
code=$(grep -vE '^[[:space:]]*#' "$df")
demo=$(grep -nE 'build:demo|--mode[ =]mock' <<<"$code" || true)
if [ -n "$demo" ]; then
	echo "FAIL: $df builds the demo: $(head -2 <<<"$demo" | tr '\n' ' ')"
	fail=1
fi
if ! grep -qE 'vite build --mode web' <<<"$code"; then
	echo "FAIL: $df does not build the real-box bundle (vite build --mode web)"
	fail=1
fi

[ "$fail" -eq 0 ] || exit 1
echo "OK: release.yml publishes, signs and releases only from a tag or a dispatch, a pull request builds without pushing, every action is pinned, and the image carries the real-box bundle"
