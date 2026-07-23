//! The onboard wizard's three commands (docs/ONBOARD.md): `onboard_status`
//! (what map + passports exist), `onboard_generate` (the four-artifact
//! bundle), `onboard_write_passport` (the one staged write).
//!
//! Every artifact is generated from the same validated inputs so the four
//! cannot disagree with each other, which is the whole point of the wizard.
//! Grammar mirrors the agent-passport spec (SPEC.md section 3: domain chars
//! `[a-z0-9.-]`, path segment chars `[a-z0-9._-]`, id <= 255 bytes) and the
//! open-TokenFuse identity map (docs/20: a pattern is a literal or a single
//! trailing `*`).

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

/// The passport schema const this wizard writes and accepts (agent-passport
/// SPEC.md section 4).
const PASSPORT_SCHEMA: &str = "taipanbox.dev/agent-passport/v0.1";

/// The attestation methods the passport schema enumerates.
const ATTESTATION_METHODS: [&str; 5] = ["none", "oidc", "spiffe-svid", "enclave-key", "mtls-cert"];

// ============================================================================
// Error
// ============================================================================

/// The plane's error: unlike the other planes' tagged unions, every onboard
/// failure carries the same two fields (the UI branches on `kind` only for
/// the overwrite confirm, see `OnboardView.tsx`). Kinds: `invalid_input`
/// (a request-level refusal), `io` (a real filesystem failure, including
/// "already exists"), `map` (a map the caller relied on is unusable).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OnboardError {
    pub kind: String,
    pub message: String,
}

impl OnboardError {
    fn invalid(message: impl Into<String>) -> Self {
        OnboardError {
            kind: "invalid_input".into(),
            message: message.into(),
        }
    }
    fn io(message: impl Into<String>) -> Self {
        OnboardError {
            kind: "io".into(),
            message: message.into(),
        }
    }
    fn map(message: impl Into<String>) -> Self {
        OnboardError {
            kind: "map".into(),
            message: message.into(),
        }
    }
}

// ============================================================================
// Requests
// ============================================================================

/// `onboard_status`'s optional overrides.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct OnboardStatusRequest {
    pub map_path: Option<String>,
    pub passports_dir: Option<String>,
}

/// One filesystem scope declared on the passport: a folder path plus a
/// mode. `mode` stays a plain `String` on the wire (the same "honesty over
/// rejection" tolerance a pass-through field gets elsewhere in this crate),
/// and is validated into the closed `read`/`write` set inside
/// `onboard_generate` rather than at the type level - so a bad value comes
/// back as a normal `OnboardError::invalid` naming the row, not an opaque
/// deserialize failure.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FsScopeDto {
    pub path: String,
    pub mode: String,
}

/// One LLM provider/model/endpoint declared on the passport (agent-passport
/// SPEC.md section 4.5, `models`). Mirrors `FsScopeDto`'s own wire/validation
/// split - a tolerant wire shape here, validated into its final form inside
/// `onboard_generate` (`validate_model_decls`) rather than at the type level.
/// Unlike `FsScopeDto`, only `provider` is required: `model` and `endpoint`
/// are honestly optional on the wire, matching the spec exactly.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelDeclDto {
    pub provider: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub endpoint: Option<String>,
}

/// `onboard_generate`'s form (docs/ONBOARD.md, exact field names).
#[derive(Debug, Clone, Deserialize)]
pub struct OnboardGenerateRequest {
    pub trust_domain: String,
    pub path: String,
    pub unit: String,
    pub owner: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub runtime: Option<String>,
    #[serde(default)]
    pub attestation_method: Option<String>,
    /// Default: `path` with `/` -> `-`.
    #[serde(default)]
    pub key_id: Option<String>,
    /// Default: the exact agent id. Literal or one trailing `*`; must match
    /// the generated agent id (a binding that misses its own agent is a
    /// misconfiguration, refused rather than emitted).
    #[serde(default)]
    pub bind_pattern: Option<String>,
    #[serde(default)]
    pub require_human_above_usd: Option<f64>,
    /// Only used when `unit` is NEW to the map.
    #[serde(default)]
    pub unit_budget_usd_month: Option<f64>,
    /// Folders this agent may access, each with a read or write mode.
    /// Default empty: no filesystem scopes is the common case, and an old
    /// frontend that omits this field entirely stays valid.
    #[serde(default)]
    pub filesystem: Vec<FsScopeDto>,
    /// LLM providers/models/endpoints this agent is declared to use
    /// (agent-passport SPEC.md section 4.5). Default empty: no declared
    /// models is the common case, and an old frontend that omits this field
    /// entirely stays valid.
    #[serde(default)]
    pub models: Vec<ModelDeclDto>,
    #[serde(default)]
    pub map_path: Option<String>,
    #[serde(default)]
    pub passports_dir: Option<String>,
}

/// `onboard_write_passport`'s args: the one staged write this wizard makes.
#[derive(Debug, Clone, Deserialize)]
pub struct OnboardWritePassportRequest {
    pub passport_json: String,
    pub passport_path: String,
    #[serde(default)]
    pub passports_dir: Option<String>,
    pub overwrite: bool,
}

// ============================================================================
// DTOs
// ============================================================================

/// One business unit from the identity map, for the form's unit picker.
#[derive(Debug, Clone, Serialize)]
pub struct UnitOptionDto {
    pub id: String,
    pub name: Option<String>,
    pub budget_usd_month: Option<f64>,
}

/// One already-provisioned passport found in the passports dir.
#[derive(Debug, Clone, Serialize)]
pub struct ProvisionedDto {
    pub agent_id: String,
    pub owner: String,
    pub file: String,
    /// Count of the passport's declared `filesystem` scopes
    /// (`PassportPeek::filesystem.len()`). Only the count is surfaced here -
    /// the individual paths/modes stay on the passport file itself, the
    /// "Provisioned passports" table shows a folder count, not the folders.
    pub filesystem_count: usize,
    /// Count of the passport's declared `models` entries
    /// (`PassportPeek::models.len()`). Only the count is surfaced here - the
    /// individual provider/model/endpoint values stay on the passport file
    /// itself; the "Provisioned passports" table shows a models count, not
    /// the models (mirrors `filesystem_count`'s exact same shape).
    pub models_count: usize,
    /// Whether any `keys[].agents` pattern in the loaded map matches this id.
    pub in_map: bool,
}

/// A passport file that could not be used, with the honest reason. Never
/// fails the listing.
#[derive(Debug, Clone, Serialize)]
pub struct SkippedDto {
    pub file: String,
    pub reason: String,
}

/// `onboard_status`'s result.
#[derive(Debug, Clone, Serialize)]
pub struct OnboardStatusDto {
    pub map_path: Option<String>,
    pub map_loaded: bool,
    pub map_error: Option<String>,
    pub units: Vec<UnitOptionDto>,
    pub passports_dir: String,
    pub passports: Vec<ProvisionedDto>,
    pub skipped: Vec<SkippedDto>,
}

/// `onboard_generate`'s result: the full artifact bundle, all text, nothing
/// written anywhere.
#[derive(Debug, Clone, Serialize)]
pub struct OnboardBundleDto {
    pub agent_id: String,
    pub passport_json: String,
    pub passport_path: String,
    /// Minted `gx_<32 hex>`. Shown once; this console never persists it.
    pub client_key_secret: String,
    /// `"<secret>:<key_id>"`, the line to append to `TOKENFUSE_CLIENT_KEYS`.
    pub client_keys_line: String,
    pub key_id: String,
    pub identity_map_fragment: String,
    pub unit_is_new: bool,
    pub wardryx_policy_stub: String,
    pub terraform_snippet: String,
}

/// `onboard_write_passport`'s result.
#[derive(Debug, Clone, Serialize)]
pub struct OnboardWriteDto {
    pub written_path: String,
    pub created_dir: bool,
}

// ============================================================================
// Grammar helpers (agent-passport SPEC.md section 3 + docs/20 patterns)
// ============================================================================

fn valid_domain(domain: &str) -> bool {
    !domain.is_empty()
        && domain
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '-')
        && domain.split('.').all(|label| !label.is_empty())
}

fn valid_path(path: &str) -> bool {
    !path.is_empty()
        && path.split('/').all(|seg| {
            !seg.is_empty()
                && seg.chars().all(|c| {
                    c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '_' || c == '-'
                })
        })
}

fn valid_agent_id(id: &str) -> bool {
    let Some(rest) = id.strip_prefix("agent://") else {
        return false;
    };
    let Some((domain, path)) = rest.split_once('/') else {
        return false;
    };
    id.len() <= 255 && valid_domain(domain) && valid_path(path)
}

/// A docs/20 pattern: a literal, or a single `*` as the final character.
fn valid_pattern(pattern: &str) -> bool {
    if pattern.is_empty() {
        return false;
    }
    match pattern.find('*') {
        None => true,
        Some(pos) => pos == pattern.len() - 1,
    }
}

fn pattern_matches(pattern: &str, value: &str) -> bool {
    match pattern.strip_suffix('*') {
        Some(prefix) => value.starts_with(prefix),
        None => pattern == value,
    }
}

fn finite_positive(v: f64, what: &str) -> Result<(), OnboardError> {
    if v.is_finite() && v > 0.0 {
        Ok(())
    } else {
        Err(OnboardError::invalid(format!(
            "{what} must be a finite number greater than zero, got `{v}`"
        )))
    }
}

/// The two modes a filesystem scope may declare (Data shape, docs/ONBOARD.md).
const FS_MODES: [&str; 2] = ["read", "write"];

/// A declared filesystem scope after validation: trimmed path, mode
/// confirmed to be exactly one of [`FS_MODES`]. Kept separate from the raw
/// wire `FsScopeDto` so the three downstream builders (the passport doc, the
/// wardryx stub, the terraform blocks) all read from the SAME validated,
/// order-preserved list rather than re-deriving it three times.
struct ValidatedFsScope {
    path: String,
    mode: String,
}

/// A declared path is honest about being a declaration, not an enforced
/// mount: absolute or relative are both accepted (the operator's own
/// filesystem, not something this wizard resolves), but it must be
/// non-empty after trimming and free of control characters (bytes < 0x20,
/// which also catches NUL) - a path that cannot even print cleanly cannot be
/// reasoned about later, in a terminal or in the generated Terraform/YAML.
fn valid_fs_scope_path(path: &str) -> bool {
    !path.is_empty() && !path.chars().any(|c| (c as u32) < 0x20)
}

/// Validate and normalize `scopes` into an order-preserved, trimmed,
/// deduplicated list - or refuse with the same `OnboardError::invalid` style
/// as every other field here. Two rows naming the same folder are refused
/// rather than silently collapsed: which mode would win is ambiguous, so
/// this is one valid shape (a path appears at most once) or a clear error
/// naming the duplicated path, never a silent merge.
fn validate_filesystem_scopes(
    scopes: &[FsScopeDto],
) -> Result<Vec<ValidatedFsScope>, OnboardError> {
    let mut out = Vec::with_capacity(scopes.len());
    let mut seen_paths: BTreeSet<String> = BTreeSet::new();
    for (idx, scope) in scopes.iter().enumerate() {
        let path = scope.path.trim().to_string();
        let raw_path = &scope.path;
        let mode = scope.mode.as_str();
        if !valid_fs_scope_path(&path) {
            return Err(OnboardError::invalid(format!(
                "filesystem scope #{idx}: path must be non-empty and free of control characters, got `{raw_path}`"
            )));
        }
        if !FS_MODES.contains(&mode) {
            return Err(OnboardError::invalid(format!(
                "filesystem scope #{idx}: mode `{mode}` is not one of {FS_MODES:?}"
            )));
        }
        if !seen_paths.insert(path.clone()) {
            return Err(OnboardError::invalid(format!(
                "filesystem scope `{path}` is declared more than once; one row per folder (which mode would win is ambiguous otherwise)"
            )));
        }
        out.push(ValidatedFsScope {
            path,
            mode: scope.mode.clone(),
        });
    }
    Ok(out)
}

/// A declared model field (`provider`, or `model`/`endpoint` when present) is
/// honest about being a declaration, not an enforced binding: it must be
/// non-empty after trimming and free of control characters (bytes < 0x20,
/// which also catches NUL) - mirrors `valid_fs_scope_path`'s exact rule,
/// applied here to `provider`/`model`/`endpoint` instead of a filesystem
/// path. Kept as its own function (rather than reusing `valid_fs_scope_path`
/// directly) so the two domains stay independently named and neither refusal
/// message has to explain itself in terms of the other's field.
fn valid_model_field(s: &str) -> bool {
    !s.is_empty() && !s.chars().any(|c| (c as u32) < 0x20)
}

/// A one-line label for a declared model triple, used in duplicate-decl
/// error messages - `anthropic (model: claude-sonnet-4-5, endpoint:
/// api.anthropic.com)`, or plain `openai` when `model`/`endpoint` are both
/// absent.
fn format_model_label(provider: &str, model: Option<&str>, endpoint: Option<&str>) -> String {
    match (model, endpoint) {
        (None, None) => provider.to_string(),
        (Some(m), None) => format!("{provider} (model: {m})"),
        (None, Some(e)) => format!("{provider} (endpoint: {e})"),
        (Some(m), Some(e)) => format!("{provider} (model: {m}, endpoint: {e})"),
    }
}

/// A declared model entry after validation: trimmed `provider` (non-empty,
/// control-char-free), and trimmed `model`/`endpoint` when present
/// (control-char-free; a blank value after trimming is treated as absent,
/// the same "blank means not provided" tolerance `display_name`/`runtime`
/// get in `onboard_generate` below). Kept separate from the raw wire
/// `ModelDeclDto` for the same reason `ValidatedFsScope` is kept separate
/// from `FsScopeDto`: the passport doc and the terraform blocks read from
/// the SAME validated, order-preserved list rather than re-deriving it twice.
struct ValidatedModelDecl {
    provider: String,
    model: Option<String>,
    endpoint: Option<String>,
}

/// Validate and normalize `decls` into an order-preserved, trimmed,
/// deduplicated list - or refuse with the same `OnboardError::invalid` style
/// as every other field here. Two rows declaring the exact same
/// (provider, model, endpoint) triple after trimming are refused rather than
/// silently collapsed - mirrors `validate_filesystem_scopes`'s duplicate
/// refusal, but keyed on the full triple rather than a single field: two
/// entries for the SAME provider with DIFFERENT models or endpoints are
/// perfectly legal (e.g. `anthropic`/`claude-sonnet-4-5` and
/// `anthropic`/`claude-opus-4-1` are two distinct, both-true declarations),
/// so only an exact repeat of all three fields is ambiguous.
fn validate_model_decls(decls: &[ModelDeclDto]) -> Result<Vec<ValidatedModelDecl>, OnboardError> {
    let mut out = Vec::with_capacity(decls.len());
    let mut seen: BTreeSet<(String, Option<String>, Option<String>)> = BTreeSet::new();
    for (idx, decl) in decls.iter().enumerate() {
        let provider = decl.provider.trim().to_string();
        let raw_provider = &decl.provider;
        if !valid_model_field(&provider) {
            return Err(OnboardError::invalid(format!(
                "model declaration #{idx}: provider must be non-empty and free of control characters, got `{raw_provider}`"
            )));
        }
        let model = match decl.model.as_deref().map(str::trim) {
            None | Some("") => None,
            Some(m) if valid_model_field(m) => Some(m.to_string()),
            Some(m) => {
                return Err(OnboardError::invalid(format!(
                    "model declaration #{idx}: model must be free of control characters, got `{m}`"
                )));
            }
        };
        let endpoint = match decl.endpoint.as_deref().map(str::trim) {
            None | Some("") => None,
            Some(e) if valid_model_field(e) => Some(e.to_string()),
            Some(e) => {
                return Err(OnboardError::invalid(format!(
                    "model declaration #{idx}: endpoint must be free of control characters, got `{e}`"
                )));
            }
        };
        let key = (provider.clone(), model.clone(), endpoint.clone());
        if !seen.insert(key) {
            return Err(OnboardError::invalid(format!(
                "model declaration `{}` is declared more than once; one row per provider/model/endpoint combination (which entry would be authoritative is ambiguous otherwise)",
                format_model_label(&provider, model.as_deref(), endpoint.as_deref())
            )));
        }
        out.push(ValidatedModelDecl {
            provider,
            model,
            endpoint,
        });
    }
    Ok(out)
}

/// Escape a free-text value for embedding in a double-quoted YAML/HCL string.
fn quoted(s: &str) -> String {
    let escaped = s.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

/// RFC 3339 UTC, seconds precision, for the passport's `created_at`.
fn now_rfc3339() -> String {
    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

/// `"YYYY-MM-DD"` UTC, for the identity-map fragment's `keys[].created`
/// (I15). Coarser than [`now_rfc3339`] on purpose - the identity map is a
/// human-edited file (docs/20), and a bare date reads better there than a
/// full timestamp; chrono is already a dependency of this crate (see
/// `now_rfc3339` above).
fn today() -> String {
    chrono::Utc::now().format("%Y-%m-%d").to_string()
}

/// Mint a fresh gateway client-key secret: `gx_` + 32 lowercase hex chars
/// (16 random bytes). Never logged, never written to disk by this plane.
fn mint_secret() -> Result<String, OnboardError> {
    let mut bytes = [0u8; 16];
    getrandom::getrandom(&mut bytes)
        .map_err(|e| OnboardError::io(format!("could not gather entropy for the secret: {e}")))?;
    let mut hex = String::with_capacity(3 + 32);
    hex.push_str("gx_");
    for b in bytes {
        use std::fmt::Write as _;
        let _ = write!(hex, "{b:02x}");
    }
    Ok(hex)
}

// ============================================================================
// Filesystem resolution
// ============================================================================

/// The identity map consulted: explicit arg, else `TOKENFUSE_IDENTITY_MAP`,
/// else none.
fn resolve_map_path(arg: &Option<String>) -> Option<PathBuf> {
    if let Some(p) = arg.as_deref() {
        let trimmed = p.trim();
        if !trimmed.is_empty() {
            return Some(PathBuf::from(trimmed));
        }
    }
    match std::env::var("TOKENFUSE_IDENTITY_MAP") {
        Ok(v) if !v.trim().is_empty() => Some(PathBuf::from(v.trim().to_string())),
        _ => None,
    }
}

/// The passports staging dir: explicit arg, else `$TAIPAN_HOME/passports`,
/// else `~/.taipan/passports`. Same `TAIPAN_HOME` contract as
/// `genaryx_core::taipan_home` (one install pointed at a scratch dir must
/// move ALL of its surfaces); duplicated here only because that helper is
/// specifically about the `environments` subdir.
fn resolve_passports_dir(arg: &Option<String>) -> PathBuf {
    if let Some(p) = arg.as_deref() {
        let trimmed = p.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }
    if let Some(home) = std::env::var_os("TAIPAN_HOME") {
        return PathBuf::from(home).join("passports");
    }
    match std::env::var_os("HOME") {
        Some(home) => PathBuf::from(home).join(".taipan").join("passports"),
        None => PathBuf::from(".taipan").join("passports"),
    }
}

// ============================================================================
// Identity-map reading (tolerant serde mirror of docs/20's JSON)
// ============================================================================

// `prefixes` is deliberately not declared: this plane never consumes it, and
// serde's default unknown-field tolerance keeps maps that carry it parsing.
#[derive(Debug, Default, Deserialize)]
struct MapFile {
    #[serde(default)]
    units: Vec<MapUnit>,
    #[serde(default)]
    keys: Vec<MapKey>,
}

#[derive(Debug, Deserialize)]
struct MapUnit {
    #[serde(default)]
    id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    budget_usd_month: Option<f64>,
}

// Only `agents` is consumed here (id-in-map matching); `key_id`/`unit` are
// deliberately not declared - serde ignores the extra keys, and declaring
// fields the code never reads is dead weight (and a dead_code lint).
#[derive(Debug, Deserialize)]
struct MapKey {
    #[serde(default)]
    agents: Vec<String>,
}

fn load_map(path: &Path) -> Result<MapFile, String> {
    let bytes =
        std::fs::read(path).map_err(|e| format!("could not read {}: {e}", path.display()))?;
    serde_json::from_slice(&bytes).map_err(|e| format!("could not parse {}: {e}", path.display()))
}

/// Whether any `keys[].agents` pattern in the map matches `agent_id`.
fn id_bound_in_map(map: &MapFile, agent_id: &str) -> bool {
    map.keys
        .iter()
        .flat_map(|k| k.agents.iter())
        .any(|p| valid_pattern(p) && pattern_matches(p, agent_id))
}

// ============================================================================
// Passport document (field order preserved via struct order)
// ============================================================================

#[derive(Serialize)]
struct PassportAttestation<'a> {
    method: &'a str,
}

/// One filesystem scope as it appears on the passport document itself -
/// borrowed strings, mirroring `PassportAttestation`'s own zero-copy shape.
#[derive(Serialize)]
struct FsScope<'a> {
    path: &'a str,
    mode: &'a str,
}

/// One declared model entry as it appears on the passport document itself -
/// borrowed strings, mirroring `FsScope`'s own zero-copy shape. `model` and
/// `endpoint` are optional on the wire too (SPEC.md section 4.5: only
/// `provider` is required), each with its own `skip_serializing_if` so an
/// entry naming neither serializes as bare `{ "provider": "..." }`.
#[derive(Serialize)]
struct PassportModel<'a> {
    provider: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    model: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    endpoint: Option<&'a str>,
}

#[derive(Serialize)]
struct PassportDoc<'a> {
    schema: &'a str,
    id: &'a str,
    owner: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    display_name: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    runtime: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    attestation: Option<PassportAttestation<'a>>,
    /// Declared filesystem access, a root-level array (Data shape,
    /// docs/ONBOARD.md). Omitted entirely when empty, so a passport with no
    /// scopes serializes byte-identically to one generated before this field
    /// existed.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    filesystem: Vec<FsScope<'a>>,
    /// Declared LLM providers/models/endpoints, a root-level array (SPEC.md
    /// section 4.5). Omitted entirely when empty, so a passport declaring no
    /// models serializes byte-identically to one generated before this field
    /// existed (mirrors `filesystem`'s own byte-identical-when-empty rule).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    models: Vec<PassportModel<'a>>,
    created_at: String,
}

/// The minimal, tolerant read side of one `filesystem` entry. Parsed so a
/// passport carrying the field peeks cleanly rather than being skipped, but
/// `path`/`mode` are still not individually consumed: the "Provisioned
/// passports" table surfaces a per-passport COUNT
/// (`ProvisionedDto::filesystem_count`, via `PassportPeek::filesystem.len()`
/// below), not the individual folders, so only the containing `Vec`'s length
/// is ever read - these two fields are not. `#[allow(dead_code)]` documents
/// that on purpose rather than leaving a future maintainer to wonder why an
/// unread field survived clippy (compare `MapKey`'s doc comment above, which
/// omits fields for the exact same dead_code reason this one instead keeps
/// and annotates).
#[derive(Deserialize)]
#[allow(dead_code)]
struct FsScopePeek {
    #[serde(default)]
    path: String,
    #[serde(default)]
    mode: String,
}

/// The minimal, tolerant read side of one `models` entry. Parsed so a
/// passport carrying the field peeks cleanly rather than being skipped, but
/// `provider`/`model`/`endpoint` are not individually consumed: the
/// "Provisioned passports" table surfaces a per-passport COUNT
/// (`ProvisionedDto::models_count`, via `PassportPeek::models.len()` below),
/// not the individual declarations - mirrors `FsScopePeek`'s exact same
/// dead-code-on-purpose shape (see its own doc comment).
#[derive(Deserialize)]
#[allow(dead_code)]
struct ModelPeek {
    #[serde(default)]
    provider: String,
    #[serde(default)]
    model: String,
    #[serde(default)]
    endpoint: String,
}

/// The minimal, tolerant read side for listing what is provisioned.
#[derive(Deserialize)]
struct PassportPeek {
    #[serde(default)]
    schema: String,
    #[serde(default)]
    id: String,
    #[serde(default)]
    owner: String,
    /// Read via `.len()` into `ProvisionedDto::filesystem_count` below - see
    /// `FsScopePeek`'s own doc comment for why ITS fields stay unread even
    /// though this one is no longer dead code.
    #[serde(default)]
    filesystem: Vec<FsScopePeek>,
    /// Read via `.len()` into `ProvisionedDto::models_count` below - see
    /// `ModelPeek`'s own doc comment for why ITS fields stay unread even
    /// though this one is no longer dead code.
    #[serde(default)]
    models: Vec<ModelPeek>,
}

// ============================================================================
// Identity-map fragment (units before keys, via struct order)
// ============================================================================

#[derive(Serialize)]
struct FragmentUnit<'a> {
    id: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    budget_usd_month: Option<f64>,
}

#[derive(Serialize)]
struct FragmentKey<'a> {
    key_id: &'a str,
    unit: &'a str,
    agents: [&'a str; 1],
    /// `"YYYY-MM-DD"`, stamped at generation time (I15 "key lifecycle
    /// health": the Credentials card's "created/age" column reads this same
    /// field back off the identity map via the gateway's `/v1/keys` report).
    created: String,
}

#[derive(Serialize)]
struct FragmentDoc<'a> {
    #[serde(skip_serializing_if = "Option::is_none")]
    units: Option<[FragmentUnit<'a>; 1]>,
    keys: [FragmentKey<'a>; 1],
}

// ============================================================================
// Commands
// ============================================================================

/// What the wizard has to work with: the identity map (units for the picker)
/// and the passports staging dir (what is already provisioned). Re-reads the
/// filesystem fresh on every call; never fails outright (a broken map is
/// reported in `map_error`, an unreadable dir in `skipped`).
pub async fn onboard_status(
    request: OnboardStatusRequest,
) -> Result<OnboardStatusDto, OnboardError> {
    let map_path = resolve_map_path(&request.map_path);
    let mut map_loaded = false;
    let mut map_error = None;
    let mut units = Vec::new();
    let mut map = None;
    if let Some(path) = &map_path {
        match load_map(path) {
            Ok(m) => {
                map_loaded = true;
                units = m
                    .units
                    .iter()
                    .map(|u| UnitOptionDto {
                        id: u.id.clone(),
                        name: u.name.clone(),
                        budget_usd_month: u.budget_usd_month,
                    })
                    .collect();
                map = Some(m);
            }
            Err(e) => map_error = Some(e),
        }
    }

    let dir = resolve_passports_dir(&request.passports_dir);
    let mut passports = Vec::new();
    let mut skipped = Vec::new();
    match std::fs::read_dir(&dir) {
        Ok(entries) => {
            let mut files: Vec<PathBuf> = entries
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().is_some_and(|ext| ext == "json"))
                .collect();
            files.sort();
            for file in files {
                let display = file.display().to_string();
                let peek: Result<PassportPeek, String> = std::fs::read(&file)
                    .map_err(|e| format!("could not read: {e}"))
                    .and_then(|b| {
                        serde_json::from_slice(&b).map_err(|e| format!("could not parse: {e}"))
                    });
                match peek {
                    Ok(p) if p.schema != PASSPORT_SCHEMA => skipped.push(SkippedDto {
                        file: display,
                        reason: format!("schema is `{}`, expected `{PASSPORT_SCHEMA}`", p.schema),
                    }),
                    Ok(p) if !valid_agent_id(&p.id) => skipped.push(SkippedDto {
                        file: display,
                        reason: format!("id `{}` is not a well-formed agent:// id", p.id),
                    }),
                    Ok(p) => {
                        let in_map = map.as_ref().is_some_and(|m| id_bound_in_map(m, &p.id));
                        let filesystem_count = p.filesystem.len();
                        let models_count = p.models.len();
                        passports.push(ProvisionedDto {
                            agent_id: p.id,
                            owner: p.owner,
                            file: display,
                            filesystem_count,
                            models_count,
                            in_map,
                        });
                    }
                    Err(reason) => skipped.push(SkippedDto {
                        file: display,
                        reason,
                    }),
                }
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // A dir that does not exist yet is a normal fresh install, not a
            // problem: the write command creates it on first use.
        }
        Err(e) => skipped.push(SkippedDto {
            file: dir.display().to_string(),
            reason: format!("could not list the passports dir: {e}"),
        }),
    }

    Ok(OnboardStatusDto {
        map_path: map_path.map(|p| p.display().to_string()),
        map_loaded,
        map_error,
        units,
        passports_dir: dir.display().to_string(),
        passports,
        skipped,
    })
}

/// Generate the consistent artifact bundle. Pure: nothing is written, and
/// the minted secret exists only in the returned value.
pub async fn onboard_generate(
    request: OnboardGenerateRequest,
) -> Result<OnboardBundleDto, OnboardError> {
    let trust_domain = request.trust_domain.trim().to_string();
    let path = request.path.trim().trim_matches('/').to_string();
    let unit = request.unit.trim().to_string();
    let owner = request.owner.trim().to_string();

    if !valid_domain(&trust_domain) {
        return Err(OnboardError::invalid(format!(
            "trust domain `{trust_domain}` is not valid: lowercase labels of [a-z0-9-] separated by dots (SPEC.md section 3)"
        )));
    }
    if !valid_path(&path) {
        return Err(OnboardError::invalid(format!(
            "agent path `{path}` is not valid: non-empty segments of [a-z0-9._-] separated by `/` (SPEC.md section 3)"
        )));
    }
    if unit.is_empty() {
        return Err(OnboardError::invalid("the unit must not be empty"));
    }
    if owner.is_empty() {
        return Err(OnboardError::invalid("the owner must not be empty"));
    }
    let attestation = match request.attestation_method.as_deref().map(str::trim) {
        None | Some("") => None,
        Some(m) if ATTESTATION_METHODS.contains(&m) => Some(m.to_string()),
        Some(other) => {
            return Err(OnboardError::invalid(format!(
                "attestation method `{other}` is not one of {ATTESTATION_METHODS:?}"
            )));
        }
    };
    if let Some(v) = request.require_human_above_usd {
        finite_positive(v, "require_human_above_usd")?;
    }
    if let Some(v) = request.unit_budget_usd_month {
        finite_positive(v, "unit_budget_usd_month")?;
    }
    let filesystem = validate_filesystem_scopes(&request.filesystem)?;
    let models = validate_model_decls(&request.models)?;

    let agent_id = format!("agent://{trust_domain}/{path}");
    if !valid_agent_id(&agent_id) {
        return Err(OnboardError::invalid(format!(
            "the generated agent id `{agent_id}` is not valid (it may be over the 255-byte cap)"
        )));
    }

    let key_id = match request.key_id.as_deref().map(str::trim) {
        None | Some("") => path.replace('/', "-"),
        Some(k) => k.to_string(),
    };
    if key_id.is_empty()
        || !key_id.chars().all(|c| {
            c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '_' || c == '-'
        })
    {
        return Err(OnboardError::invalid(format!(
            "key id `{key_id}` is not valid: non-empty, chars [a-z0-9._-] (it becomes half of a TOKENFUSE_CLIENT_KEYS entry, so `:` and `,` cannot appear)"
        )));
    }

    let bind_pattern = match request.bind_pattern.as_deref().map(str::trim) {
        None | Some("") => agent_id.clone(),
        Some(p) => p.to_string(),
    };
    if !valid_pattern(&bind_pattern) {
        return Err(OnboardError::invalid(format!(
            "bind pattern `{bind_pattern}` is not valid: a literal, or a single `*` as the final character (docs/20)"
        )));
    }
    if !pattern_matches(&bind_pattern, &agent_id) {
        return Err(OnboardError::invalid(format!(
            "bind pattern `{bind_pattern}` does not match the generated agent id `{agent_id}`; a binding that misses its own agent cannot be intended"
        )));
    }

    // The map: absent is fine (everything is new); present-but-unusable is a
    // refusal, because the caller is relying on it for unit dedup.
    let map_path = resolve_map_path(&request.map_path);
    let known_units: BTreeSet<String> = match &map_path {
        None => BTreeSet::new(),
        Some(p) => {
            let map = load_map(p).map_err(OnboardError::map)?;
            map.units.into_iter().map(|u| u.id).collect()
        }
    };
    let unit_is_new = !known_units.contains(&unit);

    let passport_doc = PassportDoc {
        schema: PASSPORT_SCHEMA,
        id: &agent_id,
        owner: &owner,
        display_name: request
            .display_name
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty()),
        runtime: request
            .runtime
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty()),
        attestation: attestation
            .as_deref()
            .map(|method| PassportAttestation { method }),
        filesystem: filesystem
            .iter()
            .map(|s| FsScope {
                path: &s.path,
                mode: &s.mode,
            })
            .collect(),
        models: models
            .iter()
            .map(|m| PassportModel {
                provider: &m.provider,
                model: m.model.as_deref(),
                endpoint: m.endpoint.as_deref(),
            })
            .collect(),
        created_at: now_rfc3339(),
    };
    let mut passport_json = serde_json::to_string_pretty(&passport_doc)
        .map_err(|e| OnboardError::io(format!("could not render the passport JSON: {e}")))?;
    passport_json.push('\n');

    let passports_dir = resolve_passports_dir(&request.passports_dir);
    let passport_path = passports_dir
        .join(format!("{}.json", path.replace('/', "-")))
        .display()
        .to_string();

    let client_key_secret = mint_secret()?;
    let client_keys_line = format!("{client_key_secret}:{key_id}");

    let fragment = FragmentDoc {
        units: unit_is_new.then(|| {
            [FragmentUnit {
                id: &unit,
                budget_usd_month: request.unit_budget_usd_month,
            }]
        }),
        keys: [FragmentKey {
            key_id: &key_id,
            unit: &unit,
            agents: [&bind_pattern],
            created: today(),
        }],
    };
    let mut identity_map_fragment = serde_json::to_string_pretty(&fragment)
        .map_err(|e| OnboardError::io(format!("could not render the map fragment: {e}")))?;
    identity_map_fragment.push('\n');

    let mut wardryx_policy_stub = String::new();
    wardryx_policy_stub.push_str(
        "# Wardryx policy stub generated by the Genaryx onboard wizard (docs/ONBOARD.md).\n# Review, adjust, and commit it next to your other policies.\n",
    );
    wardryx_policy_stub.push_str(&format!("name: onboard-{key_id}\n"));
    wardryx_policy_stub.push_str(&format!("target: {}\n", quoted(&bind_pattern)));
    if let Some(v) = request.require_human_above_usd {
        wardryx_policy_stub.push_str(&format!("require_human_above_usd: {v}\n"));
    }
    if attestation.as_deref().is_some_and(|m| m != "none") {
        wardryx_policy_stub.push_str("deny_if_unattested: true\n");
    }
    if !filesystem.is_empty() {
        wardryx_policy_stub
            .push_str("# filesystem scopes declared on the passport (informational):\n");
        for s in &filesystem {
            let mode_label = format!("{}:", s.mode);
            let path = &s.path;
            wardryx_policy_stub.push_str(&format!("#   {mode_label:<6} {path}\n"));
        }
        wardryx_policy_stub.push_str(
            "# NOTE: wardryx does not enforce filesystem paths in v1 (its policy\n# surface is deny_tool / allow_domains / require_human_above_usd /\n# deny_above_usd / max_steps / deny_if_unattested). These lines are a\n# declaration carried on the passport, not an enforced control.\n",
        );
    }

    let tf_name = key_id.replace(['-', '.'], "_");
    let mut terraform_snippet = String::new();
    terraform_snippet.push_str(
        "# Generated by the Genaryx onboard wizard (docs/ONBOARD.md). Review and commit.\n",
    );
    terraform_snippet.push_str(&format!(
        "resource \"taipan_agent_passport\" \"{tf_name}\" {{\n"
    ));
    terraform_snippet.push_str(&format!("  id    = {}\n", quoted(&agent_id)));
    terraform_snippet.push_str(&format!("  owner = {}\n", quoted(&owner)));
    if let Some(d) = passport_doc.display_name {
        terraform_snippet.push_str(&format!("  display_name = {}\n", quoted(d)));
    }
    if let Some(r) = passport_doc.runtime {
        terraform_snippet.push_str(&format!("  runtime = {}\n", quoted(r)));
    }
    if let Some(m) = attestation.as_deref() {
        terraform_snippet.push_str(&format!("  attestation_method = {}\n", quoted(m)));
    }
    for s in &filesystem {
        terraform_snippet.push_str("  filesystem {\n");
        terraform_snippet.push_str(&format!("    path = {}\n", quoted(&s.path)));
        terraform_snippet.push_str(&format!("    mode = {}\n", quoted(&s.mode)));
        terraform_snippet.push_str("  }\n");
    }
    for m in &models {
        terraform_snippet.push_str("  models {\n");
        terraform_snippet.push_str(&format!("    provider = {}\n", quoted(&m.provider)));
        if let Some(v) = m.model.as_deref() {
            terraform_snippet.push_str(&format!("    model = {}\n", quoted(v)));
        }
        if let Some(v) = m.endpoint.as_deref() {
            terraform_snippet.push_str(&format!("    endpoint = {}\n", quoted(v)));
        }
        terraform_snippet.push_str("  }\n");
    }
    terraform_snippet.push_str("}\n\n");
    terraform_snippet.push_str(&format!(
        "resource \"taipan_wardryx_policy\" \"{tf_name}\" {{\n"
    ));
    terraform_snippet.push_str(&format!(
        "  id     = {}\n",
        quoted(&format!("onboard-{key_id}"))
    ));
    terraform_snippet.push_str(&format!("  target = {}\n", quoted(&bind_pattern)));
    if let Some(v) = request.require_human_above_usd {
        terraform_snippet.push_str(&format!("  require_human_above_usd = {v}\n"));
    }
    if attestation.as_deref().is_some_and(|m| m != "none") {
        terraform_snippet.push_str("  deny_if_unattested = true\n");
    }
    terraform_snippet.push_str("}\n");

    Ok(OnboardBundleDto {
        agent_id,
        passport_json,
        passport_path,
        client_key_secret,
        client_keys_line,
        key_id,
        identity_map_fragment,
        unit_is_new,
        wardryx_policy_stub,
        terraform_snippet,
    })
}

/// Stage the passport file into the passports dir: the wizard's ONE write.
/// Content must be a real passport (schema const + well-formed id + owner),
/// the path must resolve inside the passports dir, and an existing file is
/// refused unless `overwrite` (kind `io`, message names the path).
pub async fn onboard_write_passport(
    request: OnboardWritePassportRequest,
) -> Result<OnboardWriteDto, OnboardError> {
    let peek: PassportPeek = serde_json::from_str(&request.passport_json)
        .map_err(|e| OnboardError::invalid(format!("the content is not valid JSON: {e}")))?;
    if peek.schema != PASSPORT_SCHEMA {
        return Err(OnboardError::invalid(format!(
            "the content's schema is `{}`, expected `{PASSPORT_SCHEMA}`; this command writes passports only",
            peek.schema
        )));
    }
    if !valid_agent_id(&peek.id) {
        return Err(OnboardError::invalid(format!(
            "the content's id `{}` is not a well-formed agent:// id",
            peek.id
        )));
    }
    if peek.owner.trim().is_empty() {
        return Err(OnboardError::invalid(
            "the content has no owner; a passport without an owner is not valid (SPEC.md section 4)",
        ));
    }

    let dir = resolve_passports_dir(&request.passports_dir);
    let path = PathBuf::from(&request.passport_path);
    if path
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(OnboardError::invalid(format!(
            "`{}` contains `..`; the passport must land inside the passports dir `{}`",
            path.display(),
            dir.display()
        )));
    }
    let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) else {
        return Err(OnboardError::invalid(format!(
            "`{}` has no parent directory; expected a path inside `{}`",
            path.display(),
            dir.display()
        )));
    };
    if path.file_name().is_none() {
        return Err(OnboardError::invalid(format!(
            "`{}` has no file name",
            path.display()
        )));
    }

    let created_dir = !dir.exists();
    std::fs::create_dir_all(&dir)
        .map_err(|e| OnboardError::io(format!("could not create `{}`: {e}", dir.display())))?;
    let dir_canon = dir
        .canonicalize()
        .map_err(|e| OnboardError::io(format!("could not resolve `{}`: {e}", dir.display())))?;
    let parent_canon = parent.canonicalize().map_err(|e| {
        OnboardError::invalid(format!(
            "`{}` does not resolve inside the passports dir `{}`: {e}",
            path.display(),
            dir.display()
        ))
    })?;
    if parent_canon != dir_canon {
        return Err(OnboardError::invalid(format!(
            "`{}` resolves outside the passports dir `{}`; refusing to write there",
            path.display(),
            dir.display()
        )));
    }

    let target = dir_canon.join(path.file_name().expect("checked above"));
    if target.exists() && !request.overwrite {
        return Err(OnboardError::io(format!(
            "`{}` already exists; enable overwrite to replace it",
            target.display()
        )));
    }
    std::fs::write(&target, request.passport_json.as_bytes())
        .map_err(|e| OnboardError::io(format!("could not write `{}`: {e}", target.display())))?;

    Ok(OnboardWriteDto {
        written_path: target.display().to_string(),
        created_dir,
    })
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn scratch(label: &str) -> PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "genaryx-onboard-test-{label}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn base_request() -> OnboardGenerateRequest {
        OnboardGenerateRequest {
            trust_domain: "bank.example".into(),
            path: "treasury/recon-batch".into(),
            unit: "treasury".into(),
            owner: "user://bank.example/olena".into(),
            display_name: Some("Recon batch".into()),
            runtime: Some("langgraph".into()),
            attestation_method: Some("spiffe-svid".into()),
            key_id: None,
            bind_pattern: None,
            require_human_above_usd: Some(25.0),
            unit_budget_usd_month: Some(2000.0),
            // Empty by default: no filesystem scopes is the common case.
            filesystem: Vec::new(),
            // Empty by default: no declared models is the common case.
            models: Vec::new(),
            // An explicit empty override so the test never falls through to a
            // TOKENFUSE_IDENTITY_MAP env var leaking in from the harness.
            map_path: Some(String::new()),
            passports_dir: None,
        }
    }

    fn run<T>(fut: impl std::future::Future<Output = T>) -> T {
        // The commands are async only for wrapper parity; they never await.
        futures_executor_block_on(fut)
    }

    /// A dependency-free block_on for futures that are actually synchronous.
    fn futures_executor_block_on<T>(fut: impl std::future::Future<Output = T>) -> T {
        use std::task::{Context, Poll, RawWaker, RawWakerVTable, Waker};
        fn noop_raw_waker() -> RawWaker {
            fn clone(_: *const ()) -> RawWaker {
                noop_raw_waker()
            }
            fn noop(_: *const ()) {}
            RawWaker::new(
                std::ptr::null(),
                &RawWakerVTable::new(clone, noop, noop, noop),
            )
        }
        let waker = unsafe { Waker::from_raw(noop_raw_waker()) };
        let mut cx = Context::from_waker(&waker);
        let mut fut = Box::pin(fut);
        match fut.as_mut().poll(&mut cx) {
            Poll::Ready(v) => v,
            Poll::Pending => unreachable!("onboard commands never await"),
        }
    }

    // -- grammar -------------------------------------------------------------

    #[test]
    fn segment_grammar_refusals() {
        for (field, req) in [
            ("domain", {
                let mut r = base_request();
                r.trust_domain = "Bank.example".into();
                r
            }),
            ("domain empty label", {
                let mut r = base_request();
                r.trust_domain = "bank..example".into();
                r
            }),
            ("path", {
                let mut r = base_request();
                r.path = "treasury/Recon".into();
                r
            }),
            ("path empty segment", {
                let mut r = base_request();
                r.path = "treasury//recon".into();
                r
            }),
        ] {
            let err = run(onboard_generate(req)).unwrap_err();
            assert_eq!(err.kind, "invalid_input", "{field}: {}", err.message);
        }
    }

    #[test]
    fn attestation_pattern_and_budget_refusals() {
        let mut r = base_request();
        r.attestation_method = Some("tpm".into());
        assert_eq!(run(onboard_generate(r)).unwrap_err().kind, "invalid_input");

        let mut r = base_request();
        r.bind_pattern = Some("agent://bank.example/*/recon".into());
        assert_eq!(run(onboard_generate(r)).unwrap_err().kind, "invalid_input");

        let mut r = base_request();
        r.bind_pattern = Some("agent://bank.example/fraud/*".into());
        let err = run(onboard_generate(r)).unwrap_err();
        assert!(err.message.contains("does not match"), "{}", err.message);

        let mut r = base_request();
        r.require_human_above_usd = Some(f64::NAN);
        assert_eq!(run(onboard_generate(r)).unwrap_err().kind, "invalid_input");

        let mut r = base_request();
        r.key_id = Some("has:colon".into());
        assert_eq!(run(onboard_generate(r)).unwrap_err().kind, "invalid_input");

        let mut r = base_request();
        r.path = "a".repeat(300);
        assert_eq!(run(onboard_generate(r)).unwrap_err().kind, "invalid_input");
    }

    // -- generate ------------------------------------------------------------

    #[test]
    fn the_docs_example_generates_a_consistent_bundle() {
        let bundle = run(onboard_generate(base_request())).unwrap();
        assert_eq!(bundle.agent_id, "agent://bank.example/treasury/recon-batch");
        assert_eq!(bundle.key_id, "treasury-recon-batch");
        assert!(bundle.passport_path.ends_with("treasury-recon-batch.json"));
        assert!(bundle.client_key_secret.starts_with("gx_"));
        assert_eq!(bundle.client_key_secret.len(), 35);
        assert_eq!(
            bundle.client_keys_line,
            format!("{}:{}", bundle.client_key_secret, bundle.key_id)
        );
        // No map consulted: the unit is new, so the fragment declares it.
        assert!(bundle.unit_is_new);
        let fragment: serde_json::Value =
            serde_json::from_str(&bundle.identity_map_fragment).unwrap();
        assert_eq!(fragment["units"][0]["id"], "treasury");
        assert_eq!(fragment["units"][0]["budget_usd_month"], 2000.0);
        assert_eq!(fragment["keys"][0]["key_id"], "treasury-recon-batch");
        assert_eq!(
            fragment["keys"][0]["agents"][0],
            "agent://bank.example/treasury/recon-batch"
        );
        // I15: the fragment stamps today's date on the key, matched against
        // the same `today()` this test call re-derives (a flaky midnight
        // rollover between the two calls is the one theoretical false
        // negative, exactly as any such date-boundary test would have).
        assert_eq!(fragment["keys"][0]["created"], today());
        // The passport parses and carries the exact schema + field set.
        let passport: serde_json::Value = serde_json::from_str(&bundle.passport_json).unwrap();
        assert_eq!(passport["schema"], PASSPORT_SCHEMA);
        assert_eq!(passport["id"], bundle.agent_id);
        assert_eq!(passport["attestation"]["method"], "spiffe-svid");
        assert!(passport["created_at"].as_str().unwrap().ends_with('Z'));
        // Wardryx stub and terraform both carry the same binding.
        assert!(
            bundle
                .wardryx_policy_stub
                .contains("require_human_above_usd: 25")
        );
        assert!(
            bundle
                .wardryx_policy_stub
                .contains("deny_if_unattested: true")
        );
        assert!(bundle.terraform_snippet.contains("taipan_agent_passport"));
        assert!(bundle.terraform_snippet.contains("taipan_wardryx_policy"));
        assert!(
            bundle
                .terraform_snippet
                .contains("\"agent://bank.example/treasury/recon-batch\"")
        );
    }

    #[test]
    fn two_mints_differ_and_match_the_format() {
        let a = mint_secret().unwrap();
        let b = mint_secret().unwrap();
        assert_ne!(a, b);
        for s in [&a, &b] {
            assert!(
                s.strip_prefix("gx_")
                    .unwrap()
                    .chars()
                    .all(|c| c.is_ascii_hexdigit())
            );
        }
    }

    #[test]
    fn a_known_unit_is_not_redeclared_and_an_unusable_map_refuses() {
        let dir = scratch("map");
        let map = dir.join("identity.json");
        std::fs::write(
            &map,
            r#"{"units":[{"id":"treasury","budget_usd_month":2000.0}]}"#,
        )
        .unwrap();
        let mut r = base_request();
        r.map_path = Some(map.display().to_string());
        let bundle = run(onboard_generate(r)).unwrap();
        assert!(!bundle.unit_is_new);
        let fragment: serde_json::Value =
            serde_json::from_str(&bundle.identity_map_fragment).unwrap();
        assert!(fragment.get("units").is_none());

        std::fs::write(&map, "{ not json").unwrap();
        let mut r = base_request();
        r.map_path = Some(map.display().to_string());
        assert_eq!(run(onboard_generate(r)).unwrap_err().kind, "map");
    }

    // -- filesystem scopes -----------------------------------------------------

    #[test]
    fn filesystem_scope_validation_matrix() {
        // Good: a read and a write scope on distinct paths - accepted.
        let mut r = base_request();
        r.filesystem = vec![
            FsScopeDto {
                path: "/data/reports".into(),
                mode: "read".into(),
            },
            FsScopeDto {
                path: "/data/out".into(),
                mode: "write".into(),
            },
        ];
        let bundle = run(onboard_generate(r)).unwrap();
        let passport: serde_json::Value = serde_json::from_str(&bundle.passport_json).unwrap();
        assert_eq!(passport["filesystem"][0]["path"], "/data/reports");
        assert_eq!(passport["filesystem"][0]["mode"], "read");
        assert_eq!(passport["filesystem"][1]["path"], "/data/out");
        assert_eq!(passport["filesystem"][1]["mode"], "write");

        // Bad mode: neither "read" nor "write" - named, with the row index.
        let mut r = base_request();
        r.filesystem = vec![FsScopeDto {
            path: "/data/x".into(),
            mode: "readwrite".into(),
        }];
        let err = run(onboard_generate(r)).unwrap_err();
        assert_eq!(err.kind, "invalid_input");
        assert!(err.message.contains("readwrite"), "{}", err.message);
        assert!(err.message.contains("#0"), "{}", err.message);

        // Empty path after trim.
        let mut r = base_request();
        r.filesystem = vec![FsScopeDto {
            path: "   ".into(),
            mode: "read".into(),
        }];
        assert_eq!(run(onboard_generate(r)).unwrap_err().kind, "invalid_input");

        // A control character in the path.
        let mut r = base_request();
        r.filesystem = vec![FsScopeDto {
            path: "/data/\u{0007}bad".into(),
            mode: "read".into(),
        }];
        assert_eq!(run(onboard_generate(r)).unwrap_err().kind, "invalid_input");

        // A NUL byte in the path.
        let mut r = base_request();
        r.filesystem = vec![FsScopeDto {
            path: "/data/\0bad".into(),
            mode: "read".into(),
        }];
        assert_eq!(run(onboard_generate(r)).unwrap_err().kind, "invalid_input");

        // Duplicate path (same folder declared twice) - refused, not
        // silently collapsed to whichever mode came last.
        let mut r = base_request();
        r.filesystem = vec![
            FsScopeDto {
                path: "/data/reports".into(),
                mode: "read".into(),
            },
            FsScopeDto {
                path: "/data/reports".into(),
                mode: "write".into(),
            },
        ];
        let err = run(onboard_generate(r)).unwrap_err();
        assert_eq!(err.kind, "invalid_input");
        assert!(err.message.contains("/data/reports"), "{}", err.message);
    }

    #[test]
    fn filesystem_scopes_flow_into_the_wardryx_stub_and_terraform() {
        let mut r = base_request();
        r.filesystem = vec![
            FsScopeDto {
                path: "/data/reports".into(),
                mode: "read".into(),
            },
            FsScopeDto {
                path: "/data/out".into(),
                mode: "write".into(),
            },
        ];
        let bundle = run(onboard_generate(r)).unwrap();

        assert!(
            bundle
                .wardryx_policy_stub
                .contains("filesystem scopes declared on the passport")
        );
        assert!(bundle.wardryx_policy_stub.contains("read:  /data/reports"));
        assert!(bundle.wardryx_policy_stub.contains("write: /data/out"));
        assert!(
            bundle
                .wardryx_policy_stub
                .contains("wardryx does not enforce filesystem paths in v1")
        );

        assert!(bundle.terraform_snippet.contains("filesystem {"));
        assert!(
            bundle
                .terraform_snippet
                .contains("path = \"/data/reports\"")
        );
        assert!(bundle.terraform_snippet.contains("mode = \"read\""));
        assert!(bundle.terraform_snippet.contains("path = \"/data/out\""));
        assert!(bundle.terraform_snippet.contains("mode = \"write\""));
    }

    #[test]
    fn no_filesystem_scopes_omits_the_field_and_the_stub_note() {
        // base_request() declares zero scopes - the common case. Every
        // artifact must read exactly as it did before this field existed.
        let bundle = run(onboard_generate(base_request())).unwrap();
        assert!(!bundle.passport_json.contains("filesystem"));
        assert!(!bundle.wardryx_policy_stub.contains("filesystem"));
        assert!(!bundle.terraform_snippet.contains("filesystem"));
    }

    #[test]
    fn status_tolerates_passports_with_declared_filesystem_scopes() {
        let dir = scratch("status-fs");
        let passports = dir.join("passports");
        std::fs::create_dir_all(&passports).unwrap();
        std::fs::write(
            passports.join("recon.json"),
            format!(
                r#"{{"schema":"{PASSPORT_SCHEMA}","id":"agent://bank.example/treasury/recon-batch","owner":"olena","filesystem":[{{"path":"/data/reports","mode":"read"}},{{"path":"/data/out","mode":"write"}}]}}"#
            ),
        )
        .unwrap();
        let status = run(onboard_status(OnboardStatusRequest {
            map_path: None,
            passports_dir: Some(passports.display().to_string()),
        }))
        .unwrap();
        assert_eq!(status.passports.len(), 1);
        assert!(status.skipped.is_empty());
        // The whole point of this branch: the two declared scopes surface as
        // a plain count on the DTO, for the "Provisioned passports" table.
        assert_eq!(status.passports[0].filesystem_count, 2);
    }

    // -- declared models -------------------------------------------------------

    #[test]
    fn model_decl_validation_matrix() {
        // Good: a fully-specified entry, a provider-only entry, and a second
        // entry for the SAME provider with a different model - all distinct
        // triples, all accepted.
        let mut r = base_request();
        r.models = vec![
            ModelDeclDto {
                provider: "anthropic".into(),
                model: Some("claude-sonnet-4-5".into()),
                endpoint: Some("api.anthropic.com".into()),
            },
            ModelDeclDto {
                provider: "openai".into(),
                model: None,
                endpoint: None,
            },
            ModelDeclDto {
                provider: "anthropic".into(),
                model: Some("claude-opus-4-1".into()),
                endpoint: None,
            },
        ];
        let bundle = run(onboard_generate(r)).unwrap();
        let passport: serde_json::Value = serde_json::from_str(&bundle.passport_json).unwrap();
        assert_eq!(passport["models"][0]["provider"], "anthropic");
        assert_eq!(passport["models"][0]["model"], "claude-sonnet-4-5");
        assert_eq!(passport["models"][0]["endpoint"], "api.anthropic.com");
        assert_eq!(passport["models"][1]["provider"], "openai");
        assert!(passport["models"][1].get("model").is_none());
        assert!(passport["models"][1].get("endpoint").is_none());
        assert_eq!(passport["models"][2]["provider"], "anthropic");
        assert_eq!(passport["models"][2]["model"], "claude-opus-4-1");
        assert!(passport["models"][2].get("endpoint").is_none());

        // Empty provider after trim.
        let mut r = base_request();
        r.models = vec![ModelDeclDto {
            provider: "   ".into(),
            model: None,
            endpoint: None,
        }];
        let err = run(onboard_generate(r)).unwrap_err();
        assert_eq!(err.kind, "invalid_input");
        assert!(err.message.contains("#0"), "{}", err.message);

        // A control character in the provider.
        let mut r = base_request();
        r.models = vec![ModelDeclDto {
            provider: "anthro\u{0007}pic".into(),
            model: None,
            endpoint: None,
        }];
        assert_eq!(run(onboard_generate(r)).unwrap_err().kind, "invalid_input");

        // A control character in the model.
        let mut r = base_request();
        r.models = vec![ModelDeclDto {
            provider: "anthropic".into(),
            model: Some("claude\0bad".into()),
            endpoint: None,
        }];
        assert_eq!(run(onboard_generate(r)).unwrap_err().kind, "invalid_input");

        // A control character in the endpoint.
        let mut r = base_request();
        r.models = vec![ModelDeclDto {
            provider: "anthropic".into(),
            model: None,
            endpoint: Some("api.anthropic\u{0007}.com".into()),
        }];
        assert_eq!(run(onboard_generate(r)).unwrap_err().kind, "invalid_input");

        // A blank model/endpoint (whitespace only) is tolerated as absent,
        // not refused - it is treated the same as an omitted field.
        let mut r = base_request();
        r.models = vec![ModelDeclDto {
            provider: "openai".into(),
            model: Some("   ".into()),
            endpoint: Some("".into()),
        }];
        let bundle = run(onboard_generate(r)).unwrap();
        let passport: serde_json::Value = serde_json::from_str(&bundle.passport_json).unwrap();
        assert!(passport["models"][0].get("model").is_none());
        assert!(passport["models"][0].get("endpoint").is_none());

        // Duplicate (provider, model, endpoint) triple - refused, not
        // silently collapsed.
        let mut r = base_request();
        r.models = vec![
            ModelDeclDto {
                provider: "anthropic".into(),
                model: Some("claude-sonnet-4-5".into()),
                endpoint: None,
            },
            ModelDeclDto {
                provider: "anthropic".into(),
                model: Some("claude-sonnet-4-5".into()),
                endpoint: None,
            },
        ];
        let err = run(onboard_generate(r)).unwrap_err();
        assert_eq!(err.kind, "invalid_input");
        assert!(err.message.contains("anthropic"), "{}", err.message);
        assert!(
            err.message.contains("declared more than once"),
            "{}",
            err.message
        );
    }

    #[test]
    fn model_decls_flow_into_terraform_but_never_the_wardryx_stub() {
        let mut r = base_request();
        r.models = vec![
            ModelDeclDto {
                provider: "anthropic".into(),
                model: Some("claude-sonnet-4-5".into()),
                endpoint: Some("api.anthropic.com".into()),
            },
            ModelDeclDto {
                provider: "openai".into(),
                model: None,
                endpoint: None,
            },
        ];
        let bundle = run(onboard_generate(r)).unwrap();

        assert!(bundle.terraform_snippet.contains("models {"));
        assert!(
            bundle
                .terraform_snippet
                .contains("provider = \"anthropic\"")
        );
        assert!(
            bundle
                .terraform_snippet
                .contains("model = \"claude-sonnet-4-5\"")
        );
        assert!(
            bundle
                .terraform_snippet
                .contains("endpoint = \"api.anthropic.com\"")
        );
        assert!(bundle.terraform_snippet.contains("provider = \"openai\""));

        // Models are not a wardryx policy concern (no path/tool/domain rule
        // applies to them): unlike filesystem, no note is ever added to the
        // wardryx stub for a declared model, with or without models present.
        assert!(!bundle.wardryx_policy_stub.contains("model"));
    }

    #[test]
    fn no_model_decls_omits_the_field() {
        // base_request() declares zero model entries - the common case. The
        // passport must read exactly as it did before this field existed,
        // and the wardryx stub/terraform must never mention "models" either.
        let bundle = run(onboard_generate(base_request())).unwrap();
        assert!(!bundle.passport_json.contains("models"));
        assert!(!bundle.wardryx_policy_stub.contains("models"));
        assert!(!bundle.terraform_snippet.contains("models"));
    }

    #[test]
    fn status_tolerates_passports_with_declared_models() {
        let dir = scratch("status-models");
        let passports = dir.join("passports");
        std::fs::create_dir_all(&passports).unwrap();
        std::fs::write(
            passports.join("recon.json"),
            format!(
                r#"{{"schema":"{PASSPORT_SCHEMA}","id":"agent://bank.example/treasury/recon-batch","owner":"olena","models":[{{"provider":"anthropic","model":"claude-sonnet-4-5","endpoint":"api.anthropic.com"}},{{"provider":"openai"}}]}}"#
            ),
        )
        .unwrap();
        let status = run(onboard_status(OnboardStatusRequest {
            map_path: None,
            passports_dir: Some(passports.display().to_string()),
        }))
        .unwrap();
        assert_eq!(status.passports.len(), 1);
        assert!(status.skipped.is_empty());
        // The whole point of this branch: the two declared models surface as
        // a plain count on the DTO, for the "Provisioned passports" table.
        assert_eq!(status.passports[0].models_count, 2);
    }

    // -- status --------------------------------------------------------------

    #[test]
    fn status_reports_map_passports_and_skips_tolerantly() {
        let dir = scratch("status");
        let map = dir.join("identity.json");
        std::fs::write(
            &map,
            r#"{"units":[{"id":"treasury","name":"Treasury","budget_usd_month":2000.0}],
               "keys":[{"key_id":"treasury-bots","unit":"treasury",
                        "agents":["agent://bank.example/treasury/*"]}]}"#,
        )
        .unwrap();
        let passports = dir.join("passports");
        std::fs::create_dir_all(&passports).unwrap();
        std::fs::write(
            passports.join("recon.json"),
            format!(
                r#"{{"schema":"{PASSPORT_SCHEMA}","id":"agent://bank.example/treasury/recon-batch","owner":"olena"}}"#
            ),
        )
        .unwrap();
        std::fs::write(
            passports.join("fraud.json"),
            format!(
                r#"{{"schema":"{PASSPORT_SCHEMA}","id":"agent://bank.example/fraud/bot","owner":"petro"}}"#
            ),
        )
        .unwrap();
        std::fs::write(passports.join("broken.json"), "{ nope").unwrap();

        let status = run(onboard_status(OnboardStatusRequest {
            map_path: Some(map.display().to_string()),
            passports_dir: Some(passports.display().to_string()),
        }))
        .unwrap();
        assert!(status.map_loaded);
        assert_eq!(status.map_error, None);
        assert_eq!(status.units.len(), 1);
        assert_eq!(status.units[0].id, "treasury");
        assert_eq!(status.passports.len(), 2);
        let recon = status
            .passports
            .iter()
            .find(|p| p.agent_id.contains("recon"))
            .unwrap();
        assert!(recon.in_map, "the treasury/* binding covers recon");
        assert_eq!(
            recon.filesystem_count, 0,
            "neither fixture passport here declares a filesystem field"
        );
        assert_eq!(
            recon.models_count, 0,
            "neither fixture passport here declares a models field"
        );
        let fraud = status
            .passports
            .iter()
            .find(|p| p.agent_id.contains("fraud"))
            .unwrap();
        assert!(!fraud.in_map, "nothing binds fraud/*");
        assert_eq!(fraud.filesystem_count, 0);
        assert_eq!(fraud.models_count, 0);
        assert_eq!(status.skipped.len(), 1);
        assert!(status.skipped[0].file.ends_with("broken.json"));
    }

    #[test]
    fn status_with_a_broken_map_and_a_missing_dir_still_answers() {
        let dir = scratch("brokenmap");
        let map = dir.join("identity.json");
        std::fs::write(&map, "definitely not json").unwrap();
        let status = run(onboard_status(OnboardStatusRequest {
            map_path: Some(map.display().to_string()),
            passports_dir: Some(dir.join("no-such-subdir").display().to_string()),
        }))
        .unwrap();
        assert!(!status.map_loaded);
        assert!(status.map_error.is_some());
        assert!(status.units.is_empty());
        assert!(status.passports.is_empty());
        assert!(
            status.skipped.is_empty(),
            "a missing dir is a fresh install"
        );
    }

    // -- write ---------------------------------------------------------------

    fn valid_passport_json() -> String {
        format!(
            "{{\"schema\":\"{PASSPORT_SCHEMA}\",\"id\":\"agent://bank.example/treasury/recon-batch\",\"owner\":\"olena\"}}"
        )
    }

    #[test]
    fn write_creates_the_dir_writes_once_and_needs_overwrite_to_replace() {
        let dir = scratch("write").join("passports");
        let path = dir.join("recon.json").display().to_string();
        let first = run(onboard_write_passport(OnboardWritePassportRequest {
            passport_json: valid_passport_json(),
            passport_path: path.clone(),
            passports_dir: Some(dir.display().to_string()),
            overwrite: false,
        }))
        .unwrap();
        assert!(first.created_dir);
        assert!(PathBuf::from(&first.written_path).exists());

        let again = run(onboard_write_passport(OnboardWritePassportRequest {
            passport_json: valid_passport_json(),
            passport_path: path.clone(),
            passports_dir: Some(dir.display().to_string()),
            overwrite: false,
        }))
        .unwrap_err();
        assert_eq!(again.kind, "io");
        assert!(
            again.message.contains("already exists"),
            "{}",
            again.message
        );

        let replaced = run(onboard_write_passport(OnboardWritePassportRequest {
            passport_json: valid_passport_json(),
            passport_path: path,
            passports_dir: Some(dir.display().to_string()),
            overwrite: true,
        }))
        .unwrap();
        assert!(!replaced.created_dir);
    }

    #[test]
    fn write_refuses_escapes_and_non_passport_content() {
        let root = scratch("escape");
        let dir = root.join("passports");
        let elsewhere = root.join("elsewhere");
        std::fs::create_dir_all(&elsewhere).unwrap();

        for (label, path) in [
            ("dot-dot", dir.join("..").join("evil.json")),
            ("absolute elsewhere", elsewhere.join("evil.json")),
        ] {
            let err = run(onboard_write_passport(OnboardWritePassportRequest {
                passport_json: valid_passport_json(),
                passport_path: path.display().to_string(),
                passports_dir: Some(dir.display().to_string()),
                overwrite: false,
            }))
            .unwrap_err();
            assert_eq!(err.kind, "invalid_input", "{label}: {}", err.message);
        }

        for (label, content) in [
            ("not json", "{ nope".to_string()),
            (
                "wrong schema",
                r#"{"schema":"something/else","id":"agent://a.example/b","owner":"x"}"#.to_string(),
            ),
            (
                "bad id",
                format!(r#"{{"schema":"{PASSPORT_SCHEMA}","id":"not-an-id","owner":"x"}}"#),
            ),
            (
                "no owner",
                format!(
                    r#"{{"schema":"{PASSPORT_SCHEMA}","id":"agent://a.example/b","owner":"  "}}"#
                ),
            ),
        ] {
            let err = run(onboard_write_passport(OnboardWritePassportRequest {
                passport_json: content,
                passport_path: dir.join("x.json").display().to_string(),
                passports_dir: Some(dir.display().to_string()),
                overwrite: false,
            }))
            .unwrap_err();
            assert_eq!(err.kind, "invalid_input", "{label}: {}", err.message);
        }
    }
}
