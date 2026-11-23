# The Genaryx console image (CLAUDE.md invariant 52): the console, its web UI,
# the four tools it EXECUTES rather than calls, and the two engines its proofs
# run. Published by .github/workflows/release.yml on a v* tag.
#
# The console reaches money, policy and identity over HTTP, but it reaches
# qryx, mockryx, verdryx and engram by spawning them: `engram-mcp` speaks MCP
# over stdio to a child process, and the other three are shelled out to. A
# sidecar cannot be another container's stdin, so these binaries are in this
# image or the Crypto, Drills, Quality and Memory tabs are empty.
#
# The four tools arrive as named build contexts, each at the commit the
# workflow's env block pins, so this repository is the build context and
# .dockerignore applies:
#
#   docker buildx build \
#     --build-context qryx=../qryx --build-context mockryx=../mockryx \
#     --build-context verdryx=../verdryx --build-context engram=../engram \
#     -t genaryx:dev .
#
# A build without the four contexts fails at the first COPY that names one,
# rather than producing a console with no tools.
#
# Until this file existed the image was composed by stack-k8s
# (images/console.Dockerfile there), under stack-k8s's own version.

# --- the console's own web assets -------------------------------------------
FROM node:22-slim@sha256:5ec23b7c622b00547753bc6ffd78085dd868a68aeb41c8e0a1418f033e13d96d AS web
WORKDIR /w
# The lockfile is the only source of versions, and pnpm itself is pinned by
# version and by the sha512 of its tarball (package.json `packageManager`), so
# corepack refuses another one.
COPY apps/web/package.json apps/web/pnpm-lock.yaml ./
RUN corepack enable \
 && corepack prepare "$(sed -n 's/.*"packageManager": "\(.*\)".*/\1/p' package.json)" --activate \
 && pnpm install --frozen-lockfile
COPY apps/web/ ./
# `--mode web` is which PRODUCT this bundle is: it loads .env.web, whose
# VITE_GENARYX_API=/api is what makes the bundle talk to genaryx-web. The demo
# build (`build:demo`, `--mode mock`) never reaches this image;
# scripts/release-publishes-only-on-a-tag.sh refuses either name here.
RUN pnpm exec tsc && pnpm exec vite build --mode web

# --- the console binary -----------------------------------------------------
FROM rust:1.96.1-slim@sha256:31ee7fc65186be7e0e0ccb3f2ca305f14e4739e7642a1ae65753aa5d7b874523 AS rust
WORKDIR /src
RUN apt-get update \
 && apt-get install -y --no-install-recommends pkg-config libssl-dev ca-certificates \
 && rm -rf /var/lib/apt/lists/*
COPY . ./
# The compiler in the image is the one rust-toolchain.toml names, so rustup has
# nothing to fetch during the build and a bump of one without the other stops.
RUN want=$(sed -n 's/^channel = "\(.*\)"/\1/p' rust-toolchain.toml) \
 && rustc --version | grep -q "^rustc $want " \
 && cargo build --release --locked -p genaryx-web

# --- the Go tools the console shells out to ---------------------------------
FROM golang:1.27.4-alpine@sha256:0a5f7fe796599ff487605987432e7619ae45beeb2fba4c2ce0b907879932aebf AS gotools
# A digest outranks the tag beside it, so a bump that moved the tag alone would
# keep the old Go under the new name. This line stops that build.
RUN test "$(go env GOVERSION)" = go1.27.4
ENV GOTOOLCHAIN=auto
WORKDIR /src
COPY --from=qryx . ./qryx/
COPY --from=mockryx . ./mockryx/
RUN cd qryx    && CGO_ENABLED=0 go build -trimpath -ldflags="-s -w" -o /out/qryx    ./cmd/qryx \
 && cd ../mockryx && CGO_ENABLED=0 go build -trimpath -ldflags="-s -w" -o /out/mockryx ./cmd/mockryx

# --- the two engines the console's proofs run -------------------------------
# A proof is only as good as the engine behind it (invariants 40 and 41), so
# these are copied from the images the stack runs, by digest, and named in the
# labels below. When a stack moves either plane, this repository cuts a patch
# release with the new pin.
FROM ghcr.io/taipanbox/wardryx:v2.8.0@sha256:f9b86a82114698f64a3251816ee0be6f80b4a6b2c239082b7035f1b4228f06aa AS wardryx-engine
FROM ghcr.io/taipanbox/tokenfuse:v1.25.0@sha256:3a81dbcd116557eff54aae29e7d68104af8fc788265d58e71adf3f96fea03317 AS tokenfuse-engine

# --- runtime ----------------------------------------------------------------
FROM python:3.12-slim@sha256:4840d514c583a4d3d8cad3940e68dee6f0f6f3635c12fcd0a3d74c21a5b93a54
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates libssl3 \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --system --uid 10001 --home /var/lib/stack stack \
 && mkdir -p /var/lib/stack /var/lib/stack/events \
 && chown -R stack:stack /var/lib/stack
LABEL org.opencontainers.image.title="genaryx"
# The source label links the package to this repository on its first push, as
# vouchryx's does, so a package created by this repository's workflow takes the
# repository's public visibility. `publish` still checks a stranger can pull it.
LABEL org.opencontainers.image.source="https://github.com/TAIPANBOX/genaryx"
LABEL org.opencontainers.image.licenses="Apache-2.0"
LABEL dev.taipanbox.engine.wardryx="v2.8.0@sha256:f9b86a82114698f64a3251816ee0be6f80b4a6b2c239082b7035f1b4228f06aa"
LABEL dev.taipanbox.engine.tokenfuse="v1.25.0@sha256:3a81dbcd116557eff54aae29e7d68104af8fc788265d58e71adf3f96fea03317"

# The two Python tools, installed as real commands (`verdryx`, `engram-mcp`).
# `[traces]` is pyarrow: `verdryx slo` reads the gateway's Parquet traces.
# Pinned to the version verdryx's own lock tests.
COPY --from=verdryx . /opt/verdryx/
COPY --from=engram  . /opt/engram/
RUN pip install --no-cache-dir '/opt/verdryx[traces]' 'pyarrow==25.0.1' /opt/engram \
 && rm -rf /root/.cache

COPY --from=gotools /out/qryx    /usr/local/bin/qryx
COPY --from=gotools /out/mockryx /usr/local/bin/mockryx
COPY --from=wardryx-engine /usr/local/bin/service /usr/local/bin/wardryx
COPY --from=tokenfuse-engine /usr/local/bin/tokenfuse /usr/local/bin/tokenfuse
COPY --from=rust /src/target/release/genaryx-web /usr/local/bin/genaryx-web
COPY --from=web  /w/dist /srv/ui

# Where the console keeps its own state (operator record, passkeys) and where
# the shared event volume is mounted. Both are volumes: an image that carried
# either would be an image that carried a credential.
ENV HOME=/var/lib/stack
VOLUME ["/var/lib/stack"]
EXPOSE 7420
# Numeric, because runAsNonRoot cannot verify a user name.
USER 10001:10001
# Binds wide inside the container on purpose: the network around it keeps it
# off the internet, not a loopback bind that would also hide it from probes.
ENTRYPOINT ["/usr/local/bin/genaryx-web", "serve", "--bind", "0.0.0.0:7420", "--ui", "/srv/ui"]
