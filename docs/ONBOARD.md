# Onboard: the "new agent" wizard (D15/B2)

Status: built on branch `feat/agent-onboard` (2026-07-23). Design record and
the exact command contract. Follows D15 (itrat-console/15) and the shipped B1
identity map in open TokenFuse (`tokenfuse/docs/20-identity-map.md`, PR #128).

Defensive intent, as everywhere in this stack: the wizard exists so an
operator can register THEIR OWN agent correctly in one pass. It generates
artifacts; it never talks to the network and never mutates the stack.

## What it is

Registering an agent today takes four hand-written artifacts that must agree
with each other: a Passport JSON (agent-passport v0.1), a
`TOKENFUSE_CLIENT_KEYS` entry, an identity-map fragment (docs/20), and a
Wardryx policy stub. The wizard is one form that generates all four
consistently, plus a Terraform alternative, and lists what is already
provisioned.

**Propose, never mutate** (the idryx ethos, and D15's explicit rule): the
wizard returns text blocks the operator copies and commits themselves. The
ONE convenience write is the passport file into the local staging directory
(`~/.taipan/passports/` by convention, Q3), which the operator then commits
to their own git. The wizard never edits the identity map file, never touches
env vars, never calls the Cloud, and the minted client-key secret is shown
once and never persisted by the console.

## Where it lives

- `crates/api/src/onboard/` - the plane (pure local: filesystem reads of the
  identity map + passports dir; no network, no descriptor dependency).
- Web shell: three `POST /api/command/onboard_*` arms in
  `crates/web/src/dispatch.rs`.
- Tauri shell: `apps/desktop/src-tauri/src/commands/onboard.rs` wrappers.
- UI (shared by web + Tauri): `components/OnboardView.tsx` + `lib/onboard.ts`
  + `onboardTypes.ts`, a new "Onboard" view in the shell.
- SwiftUI shell: deferred per the 2026-07-21 web-first pivot (the parity CI
  gate checks shell presence, not per-feature parity). Listed under "not
  built yet", not silently skipped.

## Command contract (exact)

All DTOs `Serialize` (+ `Deserialize` for requests), snake_case wire names.
Errors are the plane's own tagged enum, `OnboardError { kind, message }`,
serialized like every other plane's error (web = 422 body).

### `onboard_status(args: { map_path?: string, passports_dir?: string }) -> OnboardStatusDto`

```rust
pub struct OnboardStatusDto {
    /// The identity map consulted: explicit arg, else the console process's
    /// TOKENFUSE_IDENTITY_MAP env var, else None.
    pub map_path: Option<String>,
    pub map_loaded: bool,
    /// Parse/read problem, when the map path exists but is unusable. The
    /// wizard still works (free-text unit) - honest, never fatal.
    pub map_error: Option<String>,
    /// Units from the map for the picker (empty when no map).
    pub units: Vec<UnitOptionDto>,   // { id, name: Option<String>, budget_usd_month: Option<f64> }
    /// The staging dir consulted: explicit arg, else $TAIPAN_HOME/passports,
    /// else ~/.taipan/passports. Reported even when it does not exist yet.
    pub passports_dir: String,
    /// Parsed passports found there (tolerant: unparseable files land in
    /// `skipped` with the reason, never fail the listing).
    pub passports: Vec<ProvisionedDto>, // { agent_id, owner, file, in_map: bool }
    pub skipped: Vec<SkippedDto>,       // { file, reason }
}
```

`in_map`: whether any `keys[].agents` pattern in the loaded map matches the
passport's id (literal or trailing-`*` prefix, the docs/20 grammar). "Seen
live traffic yet" is deliberately NOT here (needs the Cloud; named follow-up
with the Identity-tab unit grouping).

### `onboard_generate(req: OnboardGenerateRequest) -> OnboardBundleDto`

```rust
pub struct OnboardGenerateRequest {
    pub trust_domain: String,        // "bank.example"
    pub path: String,                // "treasury/recon-batch"
    pub unit: String,
    pub owner: String,               // "user://bank.example/olena" or free text
    pub display_name: Option<String>,
    pub runtime: Option<String>,
    pub attestation_method: Option<String>, // none|oidc|spiffe-svid|enclave-key|mtls-cert
    pub key_id: Option<String>,      // default: path with '/' -> '-'
    pub bind_pattern: Option<String>,// default: the exact agent id; may end with one '*'
    pub require_human_above_usd: Option<f64>,
    pub unit_budget_usd_month: Option<f64>, // only used when the unit is NEW to the map
    pub filesystem: Vec<FsScopeDto>, // default empty; see "Filesystem access" below
    pub map_path: Option<String>,    // same resolution as onboard_status
    pub passports_dir: Option<String>,
}

pub struct OnboardBundleDto {
    pub agent_id: String,            // agent://<trust_domain>/<path>
    pub passport_json: String,       // pretty, schema taipanbox.dev/agent-passport/v0.1
    pub passport_path: String,       // <passports_dir>/<path with '/' -> '-'>.json
    pub client_key_secret: String,   // minted gx_<32 hex>, shown once, never persisted
    pub client_keys_line: String,    // "<secret>:<key_id>" (appends to TOKENFUSE_CLIENT_KEYS)
    pub key_id: String,
    pub identity_map_fragment: String, // pretty JSON: keys entry (+ units entry when the unit is new)
    pub unit_is_new: bool,
    pub wardryx_policy_stub: String, // YAML
    pub terraform_snippet: String,   // taipan_agent_passport + taipan_wardryx_policy
}
```

Validation (refuse with `OnboardError`, mirroring the tokenfuse identity-map
grammar): trust_domain segments and path segments `[a-z0-9._-]+` (path may
have `/` separators), non-empty trust_domain/path/unit/owner, attestation
from the enum above, `bind_pattern` literal or single trailing `*`,
budgets finite and > 0 when present, agent id <= 255 bytes (SPEC §3),
filesystem scopes per the rules below.

### Filesystem access

The generate form's "Filesystem access" section lets the operator declare
folders the agent may reach: a path plus a mode, zero or more rows, added
with a "+" button. Zero rows (no filesystem scopes) is the common case and
the default.

```rust
pub struct FsScopeDto { pub path: String, pub mode: String }
```

`mode` stays a plain `String` on the wire for tolerance, then is validated
into the closed `read`/`write` set inside `onboard_generate` (the same
"honesty over rejection" shape other pass-through fields get elsewhere in
this crate, but validated because this one is echoed straight into three
generated artifacts). Rules:

- each `path`: non-empty after trimming surrounding whitespace, and free of
  control characters (bytes < 0x20, which also catches NUL). Not required to
  be absolute - this is a declaration the operator writes down, not a mount
  this wizard resolves or enforces.
- each `mode`: exactly `read` or `write` (case-sensitive); anything else is
  refused, naming the bad value and the row index.
- **dedup**: two rows naming the same path (exact string, after trimming) are
  refused rather than silently collapsed - which mode would win is
  ambiguous, so the error names the duplicated path. One row per folder is
  the only valid shape.
- scopes are emitted in the order the operator declared them.

When at least one scope is declared, it flows into all three generated
artifacts:

- **Passport JSON**: a root-level `filesystem` array, `[{ "path", "mode" },
  ...]`, placed after `attestation` and before `created_at`. Omitted
  entirely (not an empty array) when no scopes are declared, so a passport
  with none is byte-identical to one generated before this field existed.
- **Wardryx policy stub**: an informational, clearly-commented block listing
  each scope, e.g. `#   read:  /data/reports`. Wardryx's policy surface
  (`deny_tool` / `allow_domains` / `require_human_above_usd` /
  `deny_above_usd` / `max_steps` / `deny_if_unattested`, see
  `~/Development/wardryx/internal/policy/policy.go`'s `Policy` struct) has no
  path field: **wardryx does not enforce filesystem paths in v1**. The stub
  says so explicitly - this is a declaration carried on the passport, not an
  enforced control, and the comment exists so nobody mistakes generating the
  note for wardryx acting on it.
- **Terraform alternative**: one nested `filesystem { path = "..." mode =
  "..." }` block per scope, inside the `taipan_agent_passport` resource,
  after `attestation_method` and before the closing brace.

The client-side form (`OnboardView.tsx` + the pure helper module
`lib/fsScopes.ts`, unit-tested) mirrors the empty-path and duplicate-path
checks so the operator sees the same problem before submitting, disabling
Generate until every row has a distinct, non-empty path - the backend above
stays the source of truth and re-validates regardless.

**Named follow-up, not silent**: `filesystem` validates today only because
the published agent-passport schema
(`~/Development/agent-passport/schemas/agent-passport.schema.json`) declares
`additionalProperties: true` at the root - a new key is accepted without a
schema change. Formalizing `filesystem` as a first-class field in
`~/Development/agent-passport/SPEC.md` is a public-spec change that needs
its own sign-off across the stack's adopters, and is deliberately OUT OF
SCOPE here; neither the spec nor the schema is touched by this feature.
Separately, `PassportPeek`'s tolerant read side (the provisioned-passports
listing) now parses a passport's `filesystem` entries so old passports
without the field keep peeking cleanly, but does not yet surface a
per-passport scope count in `ProvisionedDto`/the "Provisioned passports"
table - that table's column layout is a small, separate UI change left for
a follow-up rather than folded into this branch's generate-form scope.

### `onboard_write_passport(args: { passport_json: string, passport_path: string, passports_dir?: string, overwrite: bool }) -> OnboardWriteDto`

```rust
pub struct OnboardWriteDto { pub written_path: String, pub created_dir: bool }
```

Guardrails: `passport_path` must resolve INSIDE the passports dir (no path
escape; reject `..` and absolute paths pointing elsewhere), the content must
parse as JSON with the passport schema const + a well-formed `agent://` id
(never write a byte blob somewhere on someone's behalf), an existing file is
refused unless `overwrite`, the dir is created when missing. This is the
wizard's only write.

## Follow-ups (named, not silent)

- "Provisioned, awaiting first traffic" against the Cloud's per-agent/unit
  aggregation, when the Identity tab grows unit grouping.
- SwiftUI screen (post-pivot backlog).
- B3 (operators via customer IdP + named audit actors + WebAuthn) is a
  separate decision, not part of this branch.
- Formalizing `filesystem` as a first-class field in the public
  agent-passport SPEC.md/schema (it validates today only because the schema
  allows additional root properties) - a public-spec change needing its own
  sign-off, out of scope here.
- A per-passport filesystem scope count on `ProvisionedDto`/the "Provisioned
  passports" table (the tolerant peek already parses `filesystem` entries;
  surfacing a count is a small, separate UI change, not folded into this
  branch's generate-form scope).
