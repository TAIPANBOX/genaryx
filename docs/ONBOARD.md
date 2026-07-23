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
budgets finite and > 0 when present, agent id <= 255 bytes (SPEC §3).

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
