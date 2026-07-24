# Genaryx web console (frontend)

The browser frontend of the Genaryx console: React + TypeScript + Vite +
Tailwind, served by `genaryx-web` (`../../crates/web`) from the customer's own
box. All product logic lives in `genaryx-core`/`genaryx-api`
(`../../crates/core`, `../../crates/api`, 06 §0.9); this app is a thin shell
that renders it - every panel calls through `src/lib/transport.ts`, which
talks HTTP + Server-Sent Events to `genaryx-web`'s `POST /api/command/<name>`
and `GET /api/events`, never `fetch` directly.

## Panels

Overview, Money (TokenFuse), Policy (Wardryx) + Approvals, Identity (Idryx) +
Agent 360, Quality (Verdryx), Crypto (Qryx), Memory (Engram), Drills
(Mockryx), Remote (Distance: WireGuard + SSH + cloud inventory), Copilot
(Felyx), Pocket, Onboard, Routines, Posture, and the live Bus Explorer - one
React view per plane, in the it-rat2 design language (dark by default, full
light theme too). `WebGate` is the sign-in screen every panel sits behind
once a real `genaryx-web` is configured (local Argon2id account, optionally
also an IdP - see docs/CONSOLE-IDP.md); the mock preview below has no login
gate at all.

## Running it

```sh
pnpm install       # first run only (also approves esbuild's install script,
                    # see pnpm-workspace.yaml)
pnpm dev           # Vite dev server against a real genaryx-web on
                    # 127.0.0.1:7420 (see vite.config.ts's /api proxy)
pnpm dev:mock      # no-backend preview: every command/bus read is served
                    # from src/lib/mockPreview.ts, so the whole UI renders
                    # with nothing running behind it
pnpm build         # tsc --noEmit-equivalent (tsconfig noEmit:true) + vite
                    # build -> dist/
pnpm test          # vitest run
```

`pnpm dev` needs a real `genaryx-web` reachable at `127.0.0.1:7420` (override
with `GENARYX_WEB_ORIGIN`) - see
[`../../docs/WEB-SHELL.md`](../../docs/WEB-SHELL.md) for how to build and run
one. `pnpm dev:mock` needs nothing at all: it is the fastest way to see every
panel render without a backend, and is what a plain `vite preview` also falls
back to when no `VITE_GENARYX_API` is configured.

## Serving the built bundle

```sh
pnpm build
cd ../.. && cargo build -p genaryx-web --release
./target/release/genaryx-web serve --ui apps/web/dist --bind 127.0.0.1:7420
```

## Layout

```
apps/web/
  src/
    components/          one file per panel (MoneyView, PolicyView,
                          IdentityView, CryptoView, MemoryView, QualityView,
                          DrillsView, RemoteView, CopilotView, PocketView,
                          OnboardView, RoutinesView, PostureView,
                          OverviewView, BusExplorer, ...) plus shared chrome
                          (AppShell, AppHeader, Header, WebGate) and small
                          reusable pieces (SeverityBadge, SourceChip,
                          ConfirmButton, ...)
    lib/                  one module per plane (money.ts, policy.ts, ...):
                          fetchers/mutators over lib/transport.ts, plus pure
                          logic (access.ts, posture.ts, incidents.ts, ...)
                          kept framework-free so it stays easy to test
    lib/transport.ts      the one seam that decides HOW the UI reaches
                          genaryx-core: HTTP + SSE against genaryx-web, or the
                          mock preview - no panel talks to fetch() directly
    *Types.ts              wire types, field-for-field mirrors of the Rust
                          DTOs in crates/api/src/<plane>/{commands,env}.rs
    index.css              design tokens: dark default + light theme
  .env.web / .env.mock    build-mode env (VITE_GENARYX_API / VITE_GENARYX_MOCK)
  vite.config.ts          dev server + the /api proxy to a real genaryx-web
```
