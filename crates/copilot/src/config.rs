//! The `[copilot]` config block (docs/PHASE6.md, itrat-console/13 D13.2).
//!
//! Secrets are resolved the way every other Genaryx handle already resolves
//! them (`crates/ffi/src/*/env.rs`): from an env var or a 0600 file, NOT the
//! macOS Keychain (this codebase has no Keychain integration; the spec's
//! `keychain:` scheme is a later hardening pass). `api_key_ref` is therefore
//! `"env:VAR_NAME"` or `"file:/abs/path"`.

use std::path::Path;

use serde::Deserialize;

use crate::provider::{FuseKey, ProviderError};

/// Which provider wire to speak. `Ollama`/`LmStudio`/`OpenAiCompat`/`OpenRouter`
/// all use the one OpenAI-compatible client; `Anthropic` uses the Messages
/// client; `None` means the copilot is present but unconfigured.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    #[default]
    None,
    Ollama,
    LmStudio,
    OpenAiCompat,
    Anthropic,
    OpenRouter,
}

impl ProviderKind {
    /// The default endpoint for a provider whose config omits `base_url`. Local
    /// runtimes have well-known loopback ports; the cloud providers have fixed
    /// public bases (which the residency gate then requires opting into).
    pub fn default_base_url(self) -> Option<&'static str> {
        match self {
            ProviderKind::Ollama => Some("http://127.0.0.1:11434/v1"),
            ProviderKind::LmStudio => Some("http://127.0.0.1:1234/v1"),
            ProviderKind::Anthropic => Some("https://api.anthropic.com"),
            ProviderKind::OpenRouter => Some("https://openrouter.ai/api/v1"),
            // A bare "openai_compat"/"none" has no implied endpoint.
            ProviderKind::OpenAiCompat | ProviderKind::None => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ProviderKind::None => "none",
            ProviderKind::Ollama => "ollama",
            ProviderKind::LmStudio => "lmstudio",
            ProviderKind::OpenAiCompat => "openai_compat",
            ProviderKind::Anthropic => "anthropic",
            ProviderKind::OpenRouter => "openrouter",
        }
    }
}

/// The parsed `[copilot]` block. Every field has a default so a partial block
/// (or none at all) is valid and yields a disabled copilot.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct CopilotConfig {
    pub provider: ProviderKind,
    pub base_url: Option<String>,
    pub model: Option<String>,
    /// `"env:VAR"` or `"file:/abs/path"`. `None` is fine for local runtimes.
    pub api_key_ref: Option<String>,
    /// Hard gate, default `false`: a non-local `base_url` is refused unless this
    /// is explicitly `true` (the BYO-cloud path, D13.2).
    pub allow_non_local_endpoints: bool,
    /// `GENARYX_COPILOT_LOCAL_HOSTNAMES` (2026-09-27, invariant 14): an
    /// allow-list of exact hostnames (case-insensitive) the residency gate
    /// may resolve and check, so a Kubernetes Service name or a Compose
    /// service name (which is neither a literal address nor `localhost`, the
    /// only two things `residency::is_local_endpoint` can prove local on its
    /// own) can pass the gate without opening it to every destination the
    /// way `allow_non_local_endpoints` does. Empty (the default) keeps the
    /// gate's original behaviour: any hostname other than `localhost` is
    /// refused outright, with no DNS lookup at all. See
    /// `crate::provider::check_residency` and `crate::resolver`.
    pub local_hostnames: Vec<String>,
    /// The copilot's own daily spend ceiling, enforced via the local TokenFuse
    /// gateway in C2 (D13.3). Carried in config from C0 so the knob is stable.
    pub max_usd_per_day: f64,
    /// Bounded agent loop (D13.1): at most this many provider round trips.
    pub max_iterations: u32,
    /// Per-turn output budget handed to the provider.
    pub max_tokens: u32,
    /// The run id the copilot tags its OWN LLM calls with (C2, D13.3 self-budget):
    /// sent as an `x-fuse-run-id` header so a local TokenFuse gateway attributes
    /// and caps the copilot's inference spend like any other agent (the Breaker's
    /// 402 stops a runaway copilot). Harmless against a raw Ollama/Anthropic
    /// endpoint, which just ignores the header. Defaults to `genaryx-copilot`.
    pub run_id: String,
    /// `GENARYX_COPILOT_AGENT_ID` (2026-09-27 defect fix): an explicit override
    /// for the `x-fuse-agent-id` header every real provider now sends beside
    /// `x-fuse-run-id`, so a TokenFuse gateway running its Wardryx hook in
    /// enforce mode can identify Felyx's own calls instead of refusing them for
    /// carrying no agent identity. `None` (or empty after trimming) resolves to
    /// a default rather than disabling anything; see [`Self::resolved_agent_id`].
    pub agent_id: Option<String>,
    /// `GENARYX_COPILOT_FUSE_KEY_REF` (invariant 24): `"env:VAR"` or
    /// `"file:/abs/path"`, pointing at the gateway client key Felyx sends as
    /// `x-fuse-key`, the way `api_key_ref` points at the provider key. Only
    /// the reference lives here, never the key. `None`, empty, or a reference
    /// whose value is empty sends no such header; see
    /// [`Self::resolve_fuse_key`].
    pub fuse_key_ref: Option<String>,
}

impl Default for CopilotConfig {
    fn default() -> Self {
        Self {
            provider: ProviderKind::None,
            base_url: None,
            model: None,
            api_key_ref: None,
            allow_non_local_endpoints: false,
            local_hostnames: Vec::new(),
            max_usd_per_day: 5.0,
            max_iterations: 6,
            max_tokens: 1024,
            run_id: "genaryx-copilot".to_string(),
            agent_id: None,
            fuse_key_ref: None,
        }
    }
}

/// `GENARYX_ORG_DOMAIN`, default `local`: the same variable
/// `crates/api/src/journal.rs` reads to build the console's OWN emitted
/// `agent_id` (`agent://<org_domain>/console/<host>`). A second, independent
/// reader of the same name, in the sanctioned shape CLAUDE.md's trap 13
/// already names for `TOKENFUSE_GATEWAY_ADMIN_KEY` and `GENARYX_SCAN_TARGET`:
/// this crate does not depend on `genaryx-api`, so it cannot call
/// `journal.rs`'s own resolution, and inventing a second variable name for
/// the same concept would be the actual mistake.
const ORG_DOMAIN_VAR: &str = "GENARYX_ORG_DOMAIN";
const DEFAULT_ORG_DOMAIN: &str = "local";

fn org_domain() -> String {
    std::env::var(ORG_DOMAIN_VAR)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| DEFAULT_ORG_DOMAIN.to_string())
}

/// The estate's agent-id grammar, byte-for-byte the pattern
/// `crates/core/src/schemas/agent-event.v0.2.schema.json` enforces on the
/// wire (`^agent://[a-z0-9.-]+/[a-z0-9._/-]+$`, `maxLength` 255) and
/// `crates/core/src/command.rs`'s `console_command_line` doc comment names
/// for the console's own emitted `agent_id`. `crates/api/src/onboard/
/// commands.rs`'s `valid_agent_id` holds the identical shape for the onboard
/// wizard's agent ids; kept as its own copy here rather than reached for
/// across the crate boundary (this crate has no dependency on `genaryx-api`,
/// and `genaryx-core` is DTOs only, not this predicate).
fn is_valid_agent_id(id: &str) -> bool {
    if id.len() > 255 {
        return false;
    }
    let Some(rest) = id.strip_prefix("agent://") else {
        return false;
    };
    let Some((domain, path)) = rest.split_once('/') else {
        return false;
    };
    !domain.is_empty()
        && domain
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '-')
        && !path.is_empty()
        && path.chars().all(|c| {
            c.is_ascii_lowercase()
                || c.is_ascii_digit()
                || c == '.'
                || c == '_'
                || c == '/'
                || c == '-'
        })
}

impl CopilotConfig {
    /// Parse a `[copilot]` block out of a TOML document. A document without the
    /// block yields the default (disabled) config.
    pub fn from_toml_str(text: &str) -> Result<Self, ConfigError> {
        #[derive(Deserialize)]
        struct Doc {
            #[serde(default)]
            copilot: CopilotConfig,
        }
        let doc: Doc = toml::from_str(text).map_err(|e| ConfigError::Toml(e.to_string()))?;
        Ok(doc.copilot)
    }

    /// The `base_url` to use: the explicit one, else the provider's default.
    pub fn resolved_base_url(&self) -> Result<String, ConfigError> {
        if let Some(url) = &self.base_url {
            return Ok(url.clone());
        }
        self.provider
            .default_base_url()
            .map(str::to_string)
            .ok_or(ConfigError::MissingField("base_url"))
    }

    pub fn require_model(&self) -> Result<String, ConfigError> {
        self.model.clone().ok_or(ConfigError::MissingField("model"))
    }

    /// Resolve `api_key_ref` to the secret value, or `None` if unset. Never logs
    /// the value; a `file:` ref is read verbatim and trimmed of trailing newline.
    pub fn resolve_api_key(&self) -> Result<Option<String>, ConfigError> {
        match &self.api_key_ref {
            None => Ok(None),
            Some(reference) => SecretRef::parse(reference)
                .map_err(|_| ConfigError::BadApiKeyRef)?
                .resolve()
                .map(Some),
        }
    }

    /// The `x-fuse-agent-id` header value every real provider sends: an
    /// explicit `agent_id` when it is set and non-empty after trimming,
    /// refused (never silently substituted) when it does not match the
    /// estate's agent-id grammar; otherwise the default,
    /// `agent://<GENARYX_ORG_DOMAIN>/genaryx/felyx`. Never logged above
    /// `debug` by any caller, and never alongside the provider API key.
    pub fn resolved_agent_id(&self) -> Result<String, ConfigError> {
        match self.agent_id.as_deref().map(str::trim) {
            Some(v) if !v.is_empty() => {
                if is_valid_agent_id(v) {
                    Ok(v.to_string())
                } else {
                    Err(ConfigError::BadAgentId(v.to_string()))
                }
            }
            _ => Ok(format!("agent://{}/genaryx/felyx", org_domain())),
        }
    }

    /// `fuse_key_ref` resolved to the gateway client key Felyx sends as
    /// `x-fuse-key` (invariant 24), or `None`.
    ///
    /// Unset, empty, or a reference whose value is empty after trimming:
    /// `None`, and Felyx sends no such header, exactly as before. So a
    /// launcher can always pass `env:<NAME>` and leave `<NAME>` empty until
    /// an operator fills it. A reference that is neither `env:VAR` nor
    /// `file:/path`, a variable that is not set, a file that cannot be read,
    /// or a key [`FuseKey::new`] refuses is an error, so Felyx reports itself
    /// disabled with the reason instead of asking a keyed gateway that would
    /// answer `401` to every question. No error repeats the key, the variable
    /// name or the path: an operator may have put the key itself where the
    /// reference belongs.
    pub fn resolve_fuse_key(&self) -> Result<Option<FuseKey>, ConfigError> {
        let Some(reference) = self
            .fuse_key_ref
            .as_deref()
            .map(str::trim)
            .filter(|r| !r.is_empty())
        else {
            return Ok(None);
        };
        let value = match SecretRef::parse(reference).map_err(|_| ConfigError::BadFuseKeyRef)? {
            SecretRef::Env(var) => std::env::var(&var).map_err(|e| {
                ConfigError::FuseKeyUnavailable(match e {
                    std::env::VarError::NotPresent => {
                        "names an environment variable that is not set".to_string()
                    }
                    std::env::VarError::NotUnicode(_) => {
                        "names an environment variable that is not valid UTF-8".to_string()
                    }
                })
            })?,
            SecretRef::File(path) => std::fs::read_to_string(Path::new(&path)).map_err(|e| {
                ConfigError::FuseKeyUnavailable(format!("names a file that cannot be read ({e})"))
            })?,
        };
        let value = value.trim();
        if value.is_empty() {
            return Ok(None);
        }
        FuseKey::new(value)
            .map(Some)
            .map_err(ConfigError::BadFuseKey)
    }
}

/// A pointer to a secret, resolved at use, never stored in the config value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecretRef {
    Env(String),
    File(String),
}

impl SecretRef {
    /// Refuses without keeping what it was given: a reference that is not
    /// `env:VAR` or `file:/path` may be the secret itself, pasted in the wrong
    /// place, and an error must not repeat it (invariant 24).
    pub fn parse(reference: &str) -> Result<Self, ConfigError> {
        if let Some(var) = reference.strip_prefix("env:") {
            if var.is_empty() {
                return Err(ConfigError::BadSecretRef);
            }
            Ok(SecretRef::Env(var.to_string()))
        } else if let Some(path) = reference.strip_prefix("file:") {
            if path.is_empty() {
                return Err(ConfigError::BadSecretRef);
            }
            Ok(SecretRef::File(path.to_string()))
        } else {
            Err(ConfigError::BadSecretRef)
        }
    }

    pub fn resolve(&self) -> Result<String, ConfigError> {
        match self {
            SecretRef::Env(var) => std::env::var(var)
                .map(|v| v.trim().to_string())
                .map_err(|_| ConfigError::SecretUnavailable(format!("env var {var} is not set"))),
            SecretRef::File(path) => std::fs::read_to_string(Path::new(path))
                .map(|v| v.trim_end_matches(['\n', '\r']).to_string())
                .map_err(|e| ConfigError::SecretUnavailable(format!("reading {path}: {e}"))),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("invalid copilot config: {0}")]
    Toml(String),
    #[error("copilot config is missing required field `{0}`")]
    MissingField(&'static str),
    #[error(
        "a secret reference must be `env:VAR` or `file:/path`; what it holds is not repeated \
         here, since it may be the key itself"
    )]
    BadSecretRef,
    #[error(
        "GENARYX_COPILOT_API_KEY_REF (api_key_ref) must be `env:VAR` or `file:/path`; what it \
         holds is not repeated here, since it may be the key itself"
    )]
    BadApiKeyRef,
    #[error("copilot secret unavailable: {0}")]
    SecretUnavailable(String),
    #[error(
        "GENARYX_COPILOT_AGENT_ID `{0}` does not match the agent-id grammar \
         `agent://<domain>/<path>` (agent-passport SPEC section 3)"
    )]
    BadAgentId(String),
    #[error(
        "GENARYX_COPILOT_FUSE_KEY_REF must be `env:VAR` or `file:/path`; what it holds is \
         not repeated here, since it may be the key itself"
    )]
    BadFuseKeyRef,
    #[error("GENARYX_COPILOT_FUSE_KEY_REF {0}")]
    FuseKeyUnavailable(String),
    #[error("the gateway client key GENARYX_COPILOT_FUSE_KEY_REF points at is refused: {0}")]
    BadFuseKey(&'static str),
    #[error(transparent)]
    Provider(ProviderError),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_copilot_block_from_toml() {
        let doc = r#"
            [copilot]
            provider = "ollama"
            model = "qwen3:8b"
            max_usd_per_day = 3.0
        "#;
        let cfg = CopilotConfig::from_toml_str(doc).unwrap();
        assert_eq!(cfg.provider, ProviderKind::Ollama);
        assert_eq!(cfg.model.as_deref(), Some("qwen3:8b"));
        assert_eq!(cfg.max_usd_per_day, 3.0);
        assert!(!cfg.allow_non_local_endpoints); // default held
    }

    #[test]
    fn a_doc_without_the_block_is_the_disabled_default() {
        let cfg = CopilotConfig::from_toml_str("[something_else]\nkey = 1\n").unwrap();
        assert_eq!(cfg.provider, ProviderKind::None);
    }

    #[test]
    fn default_config_is_disabled() {
        let cfg = CopilotConfig::default();
        assert_eq!(cfg.provider, ProviderKind::None);
        assert!(!cfg.allow_non_local_endpoints);
        assert_eq!(cfg.max_iterations, 6);
    }

    #[test]
    fn ollama_defaults_to_loopback() {
        let cfg = CopilotConfig {
            provider: ProviderKind::Ollama,
            ..Default::default()
        };
        assert_eq!(
            cfg.resolved_base_url().unwrap(),
            "http://127.0.0.1:11434/v1"
        );
    }

    #[test]
    fn openai_compat_requires_explicit_base_url() {
        let cfg = CopilotConfig {
            provider: ProviderKind::OpenAiCompat,
            ..Default::default()
        };
        assert!(matches!(
            cfg.resolved_base_url(),
            Err(ConfigError::MissingField("base_url"))
        ));
    }

    #[test]
    fn secret_ref_parsing() {
        assert_eq!(
            SecretRef::parse("env:GENARYX_COPILOT_KEY").unwrap(),
            SecretRef::Env("GENARYX_COPILOT_KEY".to_string())
        );
        assert_eq!(
            SecretRef::parse("file:/etc/genaryx/copilot.key").unwrap(),
            SecretRef::File("/etc/genaryx/copilot.key".to_string())
        );
        assert!(SecretRef::parse("plain-secret").is_err());
        assert!(SecretRef::parse("env:").is_err());
    }

    #[test]
    fn env_secret_resolves_and_trims() {
        // SAFETY: single-threaded test; the var is set and read in this test only.
        unsafe {
            std::env::set_var("GENARYX_COPILOT_TEST_KEY", "  sk-abc123\n");
        }
        let resolved = SecretRef::Env("GENARYX_COPILOT_TEST_KEY".to_string())
            .resolve()
            .unwrap();
        assert_eq!(resolved, "sk-abc123");
    }

    /// One sequential test for the whole `agent_id` resolution surface, the
    /// same discipline `state.rs`'s `config_from_env_reads_the_provider_surface`
    /// already uses for `GENARYX_COPILOT_*`: `GENARYX_ORG_DOMAIN` is process-wide
    /// state, so splitting these cases across separate `#[test]` fns would race
    /// under the parallel runner. Run against the unfixed tree (no `agent_id`
    /// field, no `resolved_agent_id` method, no `BadAgentId` variant), this
    /// failed to compile with 11 errors naming exactly those three names.
    #[test]
    fn resolved_agent_id_defaults_explicit_empty_and_malformed() {
        // SAFETY: single-threaded within this test; no other test in this
        // binary reads or writes GENARYX_ORG_DOMAIN.
        unsafe {
            std::env::remove_var("GENARYX_ORG_DOMAIN");
        }

        // No override at all: the default, org domain "local".
        let cfg = CopilotConfig::default();
        assert_eq!(
            cfg.resolved_agent_id().unwrap(),
            "agent://local/genaryx/felyx"
        );

        // An explicit org domain changes the default.
        unsafe {
            std::env::set_var("GENARYX_ORG_DOMAIN", "acme.example");
        }
        assert_eq!(
            cfg.resolved_agent_id().unwrap(),
            "agent://acme.example/genaryx/felyx"
        );
        unsafe {
            std::env::remove_var("GENARYX_ORG_DOMAIN");
        }

        // An explicit, valid override wins over any default.
        let explicit = CopilotConfig {
            agent_id: Some("agent://acme.example/genaryx/felyx-2".to_string()),
            ..Default::default()
        };
        assert_eq!(
            explicit.resolved_agent_id().unwrap(),
            "agent://acme.example/genaryx/felyx-2"
        );

        // Empty (or all-whitespace) falls back to the default rather than
        // being treated as a set value.
        let empty = CopilotConfig {
            agent_id: Some("   ".to_string()),
            ..Default::default()
        };
        assert_eq!(
            empty.resolved_agent_id().unwrap(),
            "agent://local/genaryx/felyx"
        );

        // A malformed value is refused, never silently substituted.
        let malformed = CopilotConfig {
            agent_id: Some("not-an-agent-id".to_string()),
            ..Default::default()
        };
        assert!(matches!(
            malformed.resolved_agent_id(),
            Err(ConfigError::BadAgentId(v)) if v == "not-an-agent-id"
        ));

        // A value missing the required second path segment is refused too.
        let no_path = CopilotConfig {
            agent_id: Some("agent://acme.example".to_string()),
            ..Default::default()
        };
        assert!(no_path.resolved_agent_id().is_err());
    }

    /// A scratch file per case, under a directory this test binary owns.
    fn fuse_key_file(name: &str, bytes: &[u8]) -> String {
        let dir = std::env::temp_dir().join(format!("genaryx-fuse-key-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, bytes).unwrap();
        format!("file:{}", path.display())
    }

    fn with_ref(reference: &str) -> CopilotConfig {
        CopilotConfig {
            fuse_key_ref: Some(reference.to_string()),
            ..Default::default()
        }
    }

    fn sent(key: Option<FuseKey>) -> Option<String> {
        key.map(|k| k.header_value().to_str().unwrap().to_string())
    }

    /// Invariant 24, the whole resolution surface in one sequential test,
    /// because two of its cases set a process environment variable (the same
    /// discipline as `resolved_agent_id_defaults_explicit_empty_and_malformed`).
    /// Empty in every spelling sends nothing; a key is trimmed and sent as
    /// given; every refusal names the setting and repeats neither the key nor
    /// the reference.
    #[test]
    fn resolve_fuse_key_unset_empty_file_env_and_refusals() {
        // Unset, empty, blank: no header, exactly as before.
        assert!(
            CopilotConfig::default()
                .resolve_fuse_key()
                .unwrap()
                .is_none()
        );
        assert!(with_ref("").resolve_fuse_key().unwrap().is_none());
        assert!(with_ref("   ").resolve_fuse_key().unwrap().is_none());

        // A file: trimmed of surrounding whitespace, sent byte for byte.
        let r = fuse_key_file("ok", b"  sk-felyx-abc123:x~y=  \n");
        assert_eq!(
            sent(with_ref(&r).resolve_fuse_key().unwrap()).as_deref(),
            Some("sk-felyx-abc123:x~y=")
        );
        // A file holding nothing but a newline is no key, not a refusal.
        let r = fuse_key_file("blank", b"\n");
        assert!(with_ref(&r).resolve_fuse_key().unwrap().is_none());

        // An environment variable: empty is no key (a launcher passes the
        // reference always and leaves the variable empty), set is the key.
        // SAFETY: single-threaded within this test; no other test in this
        // binary reads or writes these two names.
        unsafe {
            std::env::set_var("GENARYX_COPILOT_TEST_FUSE_KEY", "");
            std::env::remove_var("GENARYX_COPILOT_TEST_FUSE_KEY_UNSET");
        }
        let env_ref = with_ref("env:GENARYX_COPILOT_TEST_FUSE_KEY");
        assert!(env_ref.resolve_fuse_key().unwrap().is_none());
        unsafe {
            std::env::set_var("GENARYX_COPILOT_TEST_FUSE_KEY", " sk-from-the-env \n");
        }
        assert_eq!(
            sent(env_ref.resolve_fuse_key().unwrap()).as_deref(),
            Some("sk-from-the-env")
        );

        // A variable that is not set: refused, naming neither the variable
        // nor anything else from the reference.
        let err = with_ref("env:GENARYX_COPILOT_TEST_FUSE_KEY_UNSET")
            .resolve_fuse_key()
            .unwrap_err();
        assert!(matches!(err, ConfigError::FuseKeyUnavailable(_)), "{err:?}");
        assert!(err.to_string().contains("GENARYX_COPILOT_FUSE_KEY_REF"));
        assert!(!err.to_string().contains("FUSE_KEY_UNSET"), "{err}");

        // A file that is not there: refused, without the path.
        let missing = std::env::temp_dir().join("genaryx-fuse-key-no-such-file-planted");
        let err = with_ref(&format!("file:{}", missing.display()))
            .resolve_fuse_key()
            .unwrap_err();
        assert!(matches!(err, ConfigError::FuseKeyUnavailable(_)), "{err:?}");
        assert!(!err.to_string().contains("no-such-file-planted"), "{err}");

        // The key pasted where the reference belongs: refused, not repeated.
        let pasted = "sk-live-pasted-in-the-wrong-place-0123";
        let err = with_ref(pasted).resolve_fuse_key().unwrap_err();
        assert!(matches!(err, ConfigError::BadFuseKeyRef), "{err:?}");
        assert!(!err.to_string().contains(pasted), "{err}");
        assert!(!format!("{err:?}").contains(pasted));

        // Keys no gateway could match as a header: refused, not repeated.
        let long = "k".repeat(crate::provider::MAX_FUSE_KEY_BYTES + 1);
        for (name, bad) in [
            ("space", "sk with a space"),
            ("control", "sk-bell\u{7}here"),
            ("non-ascii", "sk-\u{e9}t\u{e9}-key"),
            ("long", long.as_str()),
        ] {
            let r = fuse_key_file(name, bad.as_bytes());
            let err = with_ref(&r).resolve_fuse_key().unwrap_err();
            assert!(matches!(err, ConfigError::BadFuseKey(_)), "{name}: {err:?}");
            assert!(!err.to_string().contains(bad), "{name}: {err}");
        }
        let r = fuse_key_file(
            "max",
            "k".repeat(crate::provider::MAX_FUSE_KEY_BYTES).as_bytes(),
        );
        assert!(with_ref(&r).resolve_fuse_key().unwrap().is_some());

        // The config holds the reference, never the key, so printing it is safe.
        let r = fuse_key_file("printed", b"sk-never-in-a-debug-line-42");
        let cfg = with_ref(&r);
        let key = cfg.resolve_fuse_key().unwrap().unwrap();
        assert!(!format!("{cfg:?}").contains("sk-never-in-a-debug-line-42"));
        assert!(!format!("{key:?}").contains("sk-never-in-a-debug-line-42"));

        unsafe {
            std::env::remove_var("GENARYX_COPILOT_TEST_FUSE_KEY");
        }
    }

    /// Invariant 24, hostile key files: 200 seeds of printable keys, keys
    /// with spaces, control characters, non-ASCII text, bytes that are not
    /// UTF-8, and lengths either side of the limit. Each key carries a
    /// marker no message could hold by chance. Whatever the file holds, the
    /// resolution never panics, sends exactly the trimmed bytes when it
    /// accepts, and never puts the marker into an error or a `Debug` line.
    #[test]
    fn two_hundred_seeds_of_hostile_fuse_key_files_never_leak_the_key() {
        let mut state: u64 = 0x9e37_79b9_7f4a_7c15;
        let mut next = move || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let max = crate::provider::MAX_FUSE_KEY_BYTES;
        let (mut accepted, mut refused) = (0, 0);
        for seed in 0..200u64 {
            let marker = format!("FUSEMARK{seed:04}Q");
            let mut bytes: Vec<u8> = Vec::new();
            if next() % 3 == 0 {
                bytes.extend_from_slice(b" \t\n");
            }
            bytes.extend_from_slice(marker.as_bytes());
            let tail_len = match next() % 4 {
                0 => (next() % 16) as usize,
                1 => (next() % 64) as usize,
                2 => max - marker.len() - 2 + (next() % 5) as usize,
                _ => (next() % 300) as usize,
            };
            // A third of the seeds carry a tail of printable ASCII only, so
            // the accepting branch is exercised as often as the refusing one.
            let clean = next() % 3 == 0;
            for _ in 0..tail_len {
                let r = next();
                if clean {
                    bytes.push(0x21 + (r % 94) as u8);
                    continue;
                }
                match r % 10 {
                    0 => bytes.push(b' '),
                    1 => bytes.push(((r >> 8) % 0x20) as u8),
                    2 => bytes.extend_from_slice("\u{e9}".as_bytes()),
                    3 => bytes.push(0xff),
                    _ => bytes.push(0x21 + ((r >> 8) % 94) as u8),
                }
            }
            if next() % 3 == 0 {
                bytes.extend_from_slice(b"\r\n");
            }

            let r = fuse_key_file(&format!("seed-{seed}"), &bytes);
            let text = String::from_utf8(bytes.clone()).ok();
            let trimmed = text.as_deref().map(str::trim);
            match with_ref(&r).resolve_fuse_key() {
                Ok(Some(key)) => {
                    accepted += 1;
                    let t = trimmed.expect("only UTF-8 is accepted");
                    assert!(t.len() <= max && t.bytes().all(|b| b.is_ascii_graphic()));
                    assert_eq!(key.header_value().to_str().unwrap(), t, "seed {seed}");
                    assert!(!format!("{key:?}").contains(&marker), "seed {seed}");
                }
                Ok(None) => panic!("seed {seed}: a key carrying a marker read as no key"),
                Err(e) => {
                    refused += 1;
                    if let Some(t) = trimmed {
                        assert!(
                            t.len() > max || !t.bytes().all(|b| b.is_ascii_graphic()),
                            "seed {seed}: a sendable key was refused: {e}"
                        );
                    }
                    assert!(!e.to_string().contains(&marker), "seed {seed}: {e}");
                    assert!(!format!("{e:?}").contains(&marker), "seed {seed}");
                }
            }
        }
        assert!(
            accepted > 20 && refused > 20,
            "{accepted} accepted, {refused} refused"
        );
    }

    /// The provider key pasted where `GENARYX_COPILOT_API_KEY_REF` belongs
    /// (`api_key_ref`, which takes `env:VAR` or `file:/path`). Felyx's
    /// disabled reason is the construction error's Display, and the console
    /// shows it to any signed-in viewer, so neither the Display nor the Debug
    /// line may hold the key, and the reason names the setting and the two
    /// accepted forms, the way the fuse key's refusal does (invariant 24).
    #[test]
    fn an_api_key_ref_that_holds_the_key_is_refused_without_repeating_it() {
        use crate::service::CopilotService;
        use crate::tools::Clients;

        let pasted = "sk-ant-api03-pasted-where-the-reference-belongs-0123";
        let cfg = CopilotConfig {
            provider: ProviderKind::Ollama,
            base_url: Some("http://127.0.0.1:11434/v1".into()),
            model: Some("qwen2.5:3b".into()),
            api_key_ref: Some(pasted.to_string()),
            ..Default::default()
        };

        let err = cfg.resolve_api_key().unwrap_err();
        assert!(!err.to_string().contains(pasted), "{err}");
        assert!(!format!("{err:?}").contains(pasted), "{err:?}");

        let reason = match CopilotService::from_config_and_clients(&cfg, Clients::default()) {
            Ok(_) => panic!("Felyx was built from a reference it cannot read"),
            Err(e) => e.to_string(),
        };
        assert!(!reason.contains(pasted), "{reason}");
        assert!(reason.contains("GENARYX_COPILOT_API_KEY_REF"), "{reason}");
        assert!(reason.contains("env:VAR"), "{reason}");
        assert!(reason.contains("file:/path"), "{reason}");
    }
}
