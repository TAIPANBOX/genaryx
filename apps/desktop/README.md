# Genaryx desktop shell

Cross-platform Tauri 2 shell (decision D2) over `genaryx-core`. Phase-0 scope:
a working, virtualized **Bus Explorer** live-event list rendered from mock
data, in the it-rat2 design language (dark by default, full light theme too).

All product logic lives in `genaryx-core` (`../../crates/core`, 06 §0.9); this
app is a thin shell: `src-tauri/` exposes one Tauri command, `src/` renders it.

## Data is mock, on purpose

`recent_events` (in `src-tauri/src/events.rs`) returns ~40 hardcoded events
spanning all six emitting planes (tokenfuse, wardryx, engram, verdryx,
mockryx, qryx), shaped exactly like `genaryx_core::store::StoredEvent`. The
frontend (`src/lib/recentEvents.ts`) calls it through `@tauri-apps/api`, and
falls back to the same-shaped data in `src/mockData.ts` when there is no
Tauri runtime (a plain browser preview), so `pnpm build` / `pnpm preview`
always render something real-looking.

**The follow-up task**: wire the real bus. `genaryx_core::ingest::IngestService`
already has everything needed (`Store::recent_events` for the initial page,
`IngestService::subscribe()` for the live broadcast of new events). The exact
swap-in point is documented above `impl From<StoredEvent> for UiEvent` in
`src-tauri/src/events.rs`: hold an `IngestService` (or its `Store`) in Tauri
managed state, replace the `mock_events(limit)` call in the `recent_events`
command with a real `Store::recent_events` query, and forward
`IngestService::subscribe()`'s broadcast to the frontend as Tauri events.
Everything downstream of `UiEvent` (the whole `src/` frontend) already
consumes this exact shape, so that swap is the entire remaining task.

## Commands

```sh
pnpm install       # first run only (also approves esbuild's install script,
                    # see pnpm-workspace.yaml)
pnpm build         # tsc --noEmit-equivalent (tsconfig noEmit:true) + vite build -> dist/
pnpm dev           # Vite dev server alone, browser preview with mock data
pnpm tauri dev     # the real desktop app: Vite + the Rust shell together
pnpm tauri build   # a distributable desktop app bundle
```

`src-tauri/` is a standalone Cargo project (its own `Cargo.toml` /
`Cargo.lock`, not a member of the repo-root workspace), so Rust-only checks
run from inside it:

```sh
cd src-tauri
cargo check        # or `cargo build`
```

## Layout

```
apps/desktop/
  src/                    React + TypeScript frontend
    components/
      BusExplorer.tsx     the page: header, column labels, virtualized list
      EventRow.tsx        one row + its click-to-expand data/raw panel
      SeverityBadge.tsx   info/low/medium/high/critical pill
      SourceChip.tsx      per-service colored chip
      Header.tsx          app name, live event counter, theme toggle
      JsonPreview.tsx      tiny hand-rolled JSON highlighter (no dependency)
    lib/
      recentEvents.ts     invoke("recent_events") with the mock fallback
      cssVars.ts          typed helper for the CSS custom properties in index.css
    mockData.ts           browser-preview fallback data (mirrors the Rust mock)
    types.ts              UiEvent (mirrors the Rust UiEvent / StoredEvent)
    index.css             design tokens: dark default + light theme, ambient backdrop
  src-tauri/              Rust shell (Tauri 2), standalone Cargo project
    src/
      lib.rs              tauri::Builder, the `recent_events` command
      events.rs           UiEvent, the mock generator, the StoredEvent wiring point
```
