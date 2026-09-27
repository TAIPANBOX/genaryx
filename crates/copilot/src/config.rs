//! The `[copilot]` config block (docs/PHASE6.md, itrat-console/13 D13.2).
//!
//! Secrets are resolved the way every other Genaryx handle already resolves
//! them (`crates/ffi/src/*/env.rs`): from an env var or a 0600 file, NOT the
//! macOS Keychain (this codebase has no Keychain integration; the spec's
//! `keychain:` scheme is a later hardening pass). `api_key_ref` is therefore
//! `"env:VAR_NAME"` or `"file:/abs/path"`.

use std::path::Path;

use serde::Deserialize;

use crate::provider::ProviderError;

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
            Some(reference) => SecretRef::parse(reference)?.resolve().map(Some),
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
}

/// A pointer to a secret, resolved at use, never stored in the config value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecretRef {
    Env(String),
    File(String),
}

impl SecretRef {
    pub fn parse(reference: &str) -> Result<Self, ConfigError> {
        if let Some(var) = reference.strip_prefix("env:") {
            if var.is_empty() {
                return Err(ConfigError::BadSecretRef(reference.to_string()));
            }
            Ok(SecretRef::Env(var.to_string()))
        } else if let Some(path) = reference.strip_prefix("file:") {
            if path.is_empty() {
                return Err(ConfigError::BadSecretRef(reference.to_string()));
            }
            Ok(SecretRef::File(path.to_string()))
        } else {
            Err(ConfigError::BadSecretRef(reference.to_string()))
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
    #[error("api_key_ref must be `env:VAR` or `file:/path`, got `{0}`")]
    BadSecretRef(String),
    #[error("copilot secret unavailable: {0}")]
    SecretUnavailable(String),
    #[error(
        "GENARYX_COPILOT_AGENT_ID `{0}` does not match the agent-id grammar \
         `agent://<domain>/<path>` (agent-passport SPEC section 3)"
    )]
    BadAgentId(String),
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
}
