# Genaryx

Proprietary, closed-source enterprise **control room** (single pane of glass)
over the open TAIPANBOX agent-governance stack, and the one **paid** product in
the family: everything in the open stack (TokenFuse, Wardryx, Engram, Idryx,
Qryx, Verdryx, Mockryx) is and stays free; Genaryx is the secured, managed way
to run it as one product.

> Confidential. Not open source. See [LICENSE](LICENSE). The Apache-2.0 stack this
> product consumes is never relicensed (decision D3).

## What it is

A real-time surface for a CISO / Head of FinOps: fleet burn and kill switch
(**TokenFuse**), policy decisions and approvals (**Wardryx**), identity graph
(**Idryx**), crypto posture and evidence (**Qryx**), quality and cost-per-outcome
(**Verdryx**), fire-drills (**Mockryx**), and memory (**Engram**). No new backend:
the console is a privileged consumer of the per-service NDJSON event bus plus a
client of the existing HTTP APIs.

Since the 2026-07-21 web-first pivot, the product's present tense is:

- **A web console served from the customer's own box** (`genaryx-web`): the
  runtime and the console run inside the customer's perimeter, reached over the
  operator's own WireGuard tunnel (D11). Nothing about their runs, spend or
  identities travels anywhere to be displayed; it-rat.com hosts only commerce
  (accounts, payment, license, download) and has no route to the console.
- The native desktop shells this project once also shipped (a Tauri 2 shell
  for Windows/Linux, a SwiftUI shell for macOS) were removed from this repo
  with the web-only pivot; the web console above is the only console shell
  now.
- The mobile **Pocket** pager is built but deferred for distribution (Apple
  Developer account pending); it stays on the roadmap, not in the offer.

## What is built (all live-verified unless marked)

- **Phases 0-4: the console itself.** One shared Rust core (`genaryx-core`:
  ingest, store, reducers, commands, signing ceremonies, alerts, evidence,
  ToolRunner, MCP), environment connectors (FS, SSH, Cloud SSE, cloud
  inventory), and **all 14 tabs redesigned in both shells** (shared dash kit +
  FreshBadge). Phase-4 live exit gate **passed 2026-07-18**: the app raised its
  own WireGuard tunnel from the Remote panel to a Hetzner box whose control
  plane was closed to the internet (ufw), and sent a hardware-signed
  Touch-ID/ES256 kill through it. The macOS `set_addr` netmask bug on the WG
  data path was found and fixed live.
- **Phase 5 (D12): the relay and the pager.** `genaryx-relay`, a headless
  24/7 Rust service beside the stack: QR single-device pairing, APNs-push
  design, and a **bounded read surface** (money + agents) so the phone gets
  exceptions, not a firehose. TokenFuse Pocket and the Watch pair through it
  (sim-first; end-to-end exit gate passed). A finding's **provenance** is
  carried through to the phone (D14): a detection born in Idryx says so on the
  exception card, verified live idryx → Cloud → relay → phone.
- **Phase 6 (D13): Felyx.** The in-console AI copilot (`genaryx-copilot`):
  **read + propose, never act**. C0 read-only triage, C1 explanation, C2
  propose-and-confirm (every action still goes through the signed ceremony),
  C3 the intelligent-pager tie to D12 (its annotation rides the Pocket
  exception card). Provider-agnostic, local/BYO model, residency-gated;
  validated against a real cloud model with bounded tool output.
- **The web shell** (`genaryx-web` + `genaryx-api`). The command layer was
  lifted out of the (now-removed) Tauri shell into `genaryx-api`, shared
  verbatim by desktop and web at the time, so the browser console was the
  same console; the web shell is the sole survivor of that split today.
  Operator auth is one
  account per box, Argon2id, password via stdin; **loopback / tunnel bind by
  default** (a wildcard bind works but warns). Environments resolve from
  `taipan up` descriptors; `genaryx-web doctor` explains empty panels. Proven
  against the live Hetzner stack over HTTP: all eight planes ready, a real
  drill fired end to end, SSE streaming. See
  [docs/WEB-SHELL.md](docs/WEB-SHELL.md).
- **Provisioning.** Box-side WireGuard provisioner for the D11 console
  channel; the box **issues a ready device config**, the operator never
  assembles one by hand (`provisioning/`).
- **Multicloud remote (2026-07-22).** Provider-agnostic Remote panel, cloud
  inventory connectors (read-only, official-CLI ToolRunner), and the
  interactive console preview (PR #4). The WG console channel was already
  provider-agnostic.
- **Cert broker** (`cert-broker/`): ACME DNS-01 client embedded in the relay
  (design A), productionized and scripted. Its Cloudflare activation step is
  the **mobile-only** path and waits on the Apple Developer account; the
  console product needs none of it (WireGuard instead).
- **Live campaign, epoch e01 (frozen 2026-07-20).** The whole open stack under
  Genaryx on a real Hetzner box, fictional tier-1 bank `meridian.example`:
  **$4,671.02 spend / 9,501 runs / 37,596 calls / 15 runaway runs killed /
  180 incidents / $3,131.82 governed savings / 29 identities, 43 alerts.**
  e01 is the citation epoch for the articles, briefs and explainer; the site's
  enterprise gallery separately shows a 2026-07-22 run. Records, shots,
  runbook and open items live in [`live-campaign/`](live-campaign/)
  (`VERIFICATION-LOG.md`, `shots/2026-07-20/MANIFEST.md`, `OPEN-WORK.md`).
- **The "new agent" onboarding wizard (D15/B2, not yet live-verified).** One
  form generates the four artifacts registering an agent takes (Passport
  JSON, a minted `TOKENFUSE_CLIENT_KEYS` entry, an identity-map fragment for
  open TokenFuse's docs/20 map, a Wardryx policy stub, plus a Terraform
  alternative) and lists what is already provisioned. Propose-only by design:
  the operator commits everything themselves; the one convenience write is
  the passport file into the local staging dir (`~/.taipan/passports/`), and
  the minted secret is shown once, never persisted. `crates/api/src/onboard`
  + an Onboard view in the web shell; design in
  [`docs/ONBOARD.md`](docs/ONBOARD.md).
- **Console IdP login and roles (D15/B3 part 1).** `genaryx-web` verifies a
  customer's own OIDC ID-tokens offline (static JWKS, never fetched),
  alongside the existing local account. Three roles (`viewer`, `approver`,
  `admin`) gate every privileged command at the chokepoint before it
  dispatches, and a web-originated mutation now names the signed-in person in
  the audit trail instead of the box's OS account. The local account stays
  the break-glass admin. Design and the honest limits in
  [`docs/CONSOLE-IDP.md`](docs/CONSOLE-IDP.md).

## Not built yet (decided, on the roadmap)

- **Web-side signed kill** (B3 part 2): a WebAuthn passkey ceremony, so the
  console gets the same hardware-backed story the phone prototype already
  proved with Face ID.
- **Email alerts**: a thin consumer of existing bus signals, sent **by the
  box**, always an alert plus a deep link into the authenticated console,
  never a direct-execute button.
- **Store distribution** of Pocket (Apple Developer account pending; see the
  blocker note in the wiki).
- **Onboard wizard follow-up**: the "provisioned, awaiting first traffic"
  check against the Cloud's per-unit aggregation, together with Identity-tab
  unit grouping.
- **D15/B3 part 2, the WebAuthn per-action ceremony**: part 1 (OIDC login,
  roles, named audit actors) is built, see docs/CONSOLE-IDP.md. What remains
  is a per-action passkey re-sign for kill, budget, policy write, and
  approval grant; once it lands it absorbs the web-side signed kill bullet
  above.
- **Live e2e against a real gateway `/v1/keys`** (I15 "key lifecycle
  health", review-stage check): `crates/connectors/src/gateway.rs`'s DTOs
  are proven against fixtures only; a round trip against an actual running
  TokenFuse gateway is verified at review, not by a unit test.

## Layout

```
crates/
  core/         genaryx-core - all console logic (ingest, store, conform,
                commands, signing, evidence, ToolRunner, MCP).
  api/          genaryx-api - the command layer the web shell calls.
  web/          genaryx-web - the console served over HTTP from the box.
  connectors/   environment/service connectors (FS, SSH, Cloud SSE, cloud inventory).
  relay/        genaryx-relay - headless 24/7 relay for Pocket/Watch (D12).
  copilot/      Felyx (D13): read + propose, never act.
  signing/      canonical-string ceremonies (ES256 device-pairing, ML-DSA verify).
apps/
  web/          genaryx-web-ui, the browser frontend (React + Vite + TS +
                Tailwind), served by crates/web. The Tauri desktop shell and
                the SwiftUI macOS shell (and crates/ffi, its UniFFI surface)
                that once also lived in this repo were removed with the
                2026-07-21 web-only pivot.
cert-broker/    ACME DNS-01 Pocket cert broker (design A) + scripts.
provisioning/   box-side WireGuard provisioner; issues device configs.
live-campaign/  the live-validation campaign: runbooks, records, shots, open work.
docs/           PHASE0-6 scopes and exit-gate results, WEB-SHELL.md.
```

Golden truth for the event contract is vendored under
`crates/core/src/schemas/` (byte-exact copies of the open `agent-passport`
schemas) and `crates/core/tests/fixtures/` (real campaign NDJSON).

## Build

```sh
cargo build            # workspace (core + api + connectors + signing + relay + copilot + web)
cargo test             # unit + golden conformance tests
cargo fmt --all        # formatting (pinned toolchain, see rust-toolchain.toml)
cargo clippy --all-targets --all-features
```

Web console (browser UI bundle, then the server):

```sh
cd apps/web && pnpm build     # -> apps/web/dist
cd ../..    && cargo build -p genaryx-web --release
```

## Architecture source of truth

The plan lives in [`~/Development/itrat-console`](../itrat-console): files
00-09 (product, architecture, contracts, UX, roadmap D1-D8), **12** (D12
relay/Pocket) and **13** (D13 Felyx). Per-phase scopes and exit-gate results
are under [`docs/`](docs/); the web shell's operational truth is
[docs/WEB-SHELL.md](docs/WEB-SHELL.md). Read those before architectural
changes.

## Process

Architect (Fable 5 / Opus) writes specs and reviews every diff; implementation
by Sonnet 5 subagents against self-contained specs. Feature parity between the
shells is a CI checklist (a feature exists only when it lands in the core and
every shipping shell within the same phase).

**Do not push without explicit sign-off. No publicity until Yurii's explicit call.**
