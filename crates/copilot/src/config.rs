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
    /// The copilot's own daily spend ceiling, enforced via the local TokenFuse
    /// gateway in C2 (D13.3). Carried in config from C0 so the knob is stable.
    pub max_usd_per_day: f64,
    /// Bounded agent loop (D13.1): at most this many provider round trips.
    pub max_iterations: u32,
    /// Per-turn output budget handed to the provider.
    pub max_tokens: u32,
}

impl Default for CopilotConfig {
    fn default() -> Self {
        Self {
            provider: ProviderKind::None,
            base_url: None,
            model: None,
            api_key_ref: None,
            allow_non_local_endpoints: false,
            max_usd_per_day: 5.0,
            max_iterations: 6,
            max_tokens: 1024,
        }
    }
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
}
