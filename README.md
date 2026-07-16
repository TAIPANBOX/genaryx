# Genaryx

Proprietary, closed-source desktop **control room** (single pane of glass) over
the open TAIPANBOX agent-governance stack. One shared Rust core, two thin shells:
native **SwiftUI** (macOS) and **Tauri 2** (cross-platform).

> Confidential. Not open source. See [LICENSE](LICENSE). The Apache-2.0 stack this
> product consumes is never relicensed (decision D3).

## What it is

A real-time surface for a CISO / Head of FinOps: fleet burn and kill switch
(**TokenFuse**), policy decisions and approvals (**Wardryx**), identity graph
(**Idryx**), crypto posture and evidence (**Qryx**), quality and cost-per-outcome
(**Verdryx**), fire-drills (**Mockryx**), and memory (**Engram**). No new backend:
the console is a privileged local consumer of the per-service NDJSON event bus plus
a client of the existing HTTP APIs.

## Architecture source of truth

The full plan lives in [`~/Development/itrat-console`](../itrat-console) (files
00-09). Read those before making architectural changes. Key anchors:

- **06** app architecture (core modules, security model)
- **07** integration contracts (real endpoints/events, extracted from code)
- **08** functional/UX spec (personas, screens, feature catalog by MVP/v1/ENT)
- **09** roadmap (phases F0-F5) and process

## Layout

```
crates/
  core/         genaryx-core — ALL logic: ingest, store, reducers, commands,
                signing ceremonies, alerts, evidence, ToolRunner, MCP.
  connectors/   environment/service connectors (FS, SSH, Cloud SSE, cloud).
  signing/      canonical-string signing ceremonies (ES256 device-pairing, ML-DSA).
apps/
  macos/        SwiftUI shell (UniFFI / XCFramework over the core).   [Phase 0+]
  desktop/      Tauri 2 shell (React + Vite + TS + Tailwind).         [Phase 0+]
docs/
  PHASE0.md     Phase-0 scope, spike log, and verdicts.
```

Golden truth for the event contract is vendored under
`crates/core/src/schemas/` (byte-exact copies of the open `agent-passport`
schemas) and `crates/core/tests/fixtures/` (real campaign NDJSON).

## Build

```sh
cargo build            # workspace (core + connectors + signing)
cargo test             # unit + golden conformance tests
cargo fmt --all        # formatting
cargo clippy --all-targets --all-features
```

macOS shell (needs full Xcode; the box has it but the active dir is CLT):

```sh
export DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer
```

## Process

Architect (Fable 5 / Opus) writes specs and reviews every diff; implementation by
Sonnet 5 subagents against self-contained specs. Feature parity between the two
shells is a CI checklist (a feature exists only when it lands in the core and both
shells within the same phase). See [`docs/PHASE0.md`](docs/PHASE0.md).

**Do not push without explicit sign-off. No publicity until Yurii's explicit call.**
