//! Delegation-revoke environment resolution: which vouchryx to call, and
//! with which bearer key, for `delegation_revoke`.
//!
//! Unlike every other plane's `env` (a best-effort discovery that degrades
//! to a clean "no environment" state), this one is resolved EAGERLY, once,
//! at process startup, and an inconsistent pair of variables refuses to
//! start the process rather than degrading: a revoke key is a credential
//! that cuts an agent's authority, and a console that silently ran with half
//! of that configuration (a URL with no key, or a key nobody can reach)
//! would be worse than one that refused to come up and said why.
//!
//! - **Neither variable set**: [`RevokeConfig::NotConfigured`], a normal,
//!   legitimate state. `delegation_revoke` answers a named refusal and
//!   calls nobody; the console otherwise starts and serves everything else.
//! - **Both set, and the key file reads**: [`RevokeConfig::Configured`].
//! - **One set without the other, or the key file cannot be read (missing,
//!   a directory, unreadable, or empty after trimming)**: [`Err`]. The
//!   caller (`crates/web/src/main.rs`'s `Cmd::Serve`) treats this as fatal,
//!   the same posture as a bind address already in use: print why, exit
//!   non-zero, never serve half-configured.

use std::path::Path;

/// `GENARYX_VOUCHRYX_URL`: the http(s) URL of vouchryx, e.g.
/// `http://127.0.0.1:4310`.
pub const URL_VAR: &str = "GENARYX_VOUCHRYX_URL";

/// `GENARYX_VOUCHRYX_REVOKE_KEY_FILE`: a path. The file's content, trimmed,
/// is the bearer key vouchryx's own `VOUCHRYX_REVOKE_KEYS` was started with.
pub const KEY_FILE_VAR: &str = "GENARYX_VOUCHRYX_REVOKE_KEY_FILE";

/// The revoke bearer key, wrapped so it cannot be printed by accident.
/// `Debug` is redacted; [`RevokeKey::as_str`] is the one way to reach the
/// real bytes, and its one caller is the `Authorization` header
/// `commands.rs` builds.
#[derive(Clone, PartialEq, Eq)]
pub struct RevokeKey(String);

impl RevokeKey {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for RevokeKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RevokeKey(redacted)")
    }
}

/// Where this console sends a revocation, resolved once at startup.
#[derive(Debug, Clone)]
pub enum RevokeConfig {
    /// Neither variable is set. `delegation_revoke` refuses, naming both.
    NotConfigured,
    Configured {
        /// Trimmed of a trailing slash, so callers can `format!("{url}/v1/revoke")`.
        vouchryx_url: String,
        revoke_key: RevokeKey,
    },
}

/// Resolve [`RevokeConfig`] from the process environment, reading the key
/// file the one time this is called (startup). See the module doc for what
/// each outcome means.
pub fn resolve_from_env() -> Result<RevokeConfig, String> {
    resolve_from(
        std::env::var(URL_VAR).ok(),
        std::env::var(KEY_FILE_VAR).ok(),
    )
}

/// Testable core of [`resolve_from_env`], taking the (already-read) variable
/// values directly - the same reason `money::env::env_fallback_from` does:
/// `cargo test`'s parallel-by-default threads make mutating real process
/// environment inherently racy across a shared process. The key file itself
/// is still read from the real filesystem: a path is a value, not process
/// state, and tests point it at their own temp files.
fn resolve_from(url: Option<String>, key_file: Option<String>) -> Result<RevokeConfig, String> {
    let url = url.filter(|s| !s.trim().is_empty());
    let key_file = key_file.filter(|s| !s.trim().is_empty());
    match (url, key_file) {
        (None, None) => Ok(RevokeConfig::NotConfigured),
        (Some(url), Some(path)) => {
            let url = validate_url(&url)?;
            let key = read_key_file(Path::new(&path))?;
            Ok(RevokeConfig::Configured {
                vouchryx_url: url,
                revoke_key: RevokeKey(key),
            })
        }
        (Some(_), None) => Err(format!(
            "{URL_VAR} is set but {KEY_FILE_VAR} is not: delegation_revoke needs both or neither"
        )),
        (None, Some(_)) => Err(format!(
            "{KEY_FILE_VAR} is set but {URL_VAR} is not: delegation_revoke needs both or neither"
        )),
    }
}

/// `url`, trimmed of a trailing slash, or an error naming why it is not a
/// usable http(s) URL. Not a full RFC 3986 parse (this workspace's other
/// `_URL` variables, `WARDRYX_URL`/`TOKENFUSE_CLOUD_URL`, are not validated
/// at all): just enough that a copy-pasted key path or a bare host without a
/// scheme fails at startup rather than on the first revocation attempt.
fn validate_url(url: &str) -> Result<String, String> {
    let trimmed = url.trim();
    if !(trimmed.starts_with("http://") || trimmed.starts_with("https://")) {
        return Err(format!(
            "{URL_VAR} ({url}) must start with http:// or https://"
        ));
    }
    Ok(trimmed.trim_end_matches('/').to_string())
}

/// Read and trim the revoke key file. An unreadable file (missing, a
/// directory, permissions) or one that is empty after trimming is a startup
/// error, never a silent `NotConfigured`: the operator asked for revocation
/// by setting both variables, and a key that cannot be read is a
/// configuration mistake to fix, not a feature to quietly turn off.
fn read_key_file(path: &Path) -> Result<String, String> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| format!("{KEY_FILE_VAR} ({}) cannot be read: {e}", path.display()))?;
    let key = raw.trim().to_string();
    if key.is_empty() {
        return Err(format!("{KEY_FILE_VAR} ({}) is empty", path.display()));
    }
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn unique_path(tag: &str) -> std::path::PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "genaryx-delegation-env-test-{tag}-{}-{n}",
            std::process::id()
        ))
    }

    fn write(path: &Path, body: &str) {
        std::fs::write(path, body).expect("write fixture file");
    }

    #[test]
    fn both_unset_is_not_configured_and_needs_no_file() {
        assert!(matches!(
            resolve_from(None, None),
            Ok(RevokeConfig::NotConfigured)
        ));
    }

    #[test]
    fn blank_values_count_as_unset() {
        assert!(matches!(
            resolve_from(Some("   ".into()), Some("\t".into())),
            Ok(RevokeConfig::NotConfigured)
        ));
    }

    #[test]
    fn both_set_and_readable_resolves_configured_with_a_trimmed_key() {
        let path = unique_path("happy");
        write(&path, "  s3cr3t-key-bytes  \n");
        let resolved = resolve_from(
            Some("http://127.0.0.1:4310".into()),
            Some(path.to_string_lossy().into_owned()),
        )
        .expect("both set and readable must resolve");
        match resolved {
            RevokeConfig::Configured {
                vouchryx_url,
                revoke_key,
            } => {
                assert_eq!(vouchryx_url, "http://127.0.0.1:4310");
                assert_eq!(revoke_key.as_str(), "s3cr3t-key-bytes");
            }
            RevokeConfig::NotConfigured => panic!("must be Configured"),
        }
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_trailing_slash_on_the_url_is_trimmed() {
        let path = unique_path("trailing-slash");
        write(&path, "key");
        let resolved = resolve_from(
            Some("https://vouchryx.local/".into()),
            Some(path.to_string_lossy().into_owned()),
        )
        .expect("must resolve");
        match resolved {
            RevokeConfig::Configured { vouchryx_url, .. } => {
                assert_eq!(vouchryx_url, "https://vouchryx.local");
            }
            RevokeConfig::NotConfigured => panic!("must be Configured"),
        }
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn url_without_the_key_file_refuses_to_start() {
        let err = resolve_from(Some("http://127.0.0.1:4310".into()), None).unwrap_err();
        assert!(err.contains(URL_VAR), "{err}");
        assert!(err.contains(KEY_FILE_VAR), "{err}");
    }

    #[test]
    fn key_file_without_the_url_refuses_to_start() {
        let path = unique_path("key-alone");
        write(&path, "key");
        let err = resolve_from(None, Some(path.to_string_lossy().into_owned())).unwrap_err();
        assert!(err.contains(URL_VAR), "{err}");
        assert!(err.contains(KEY_FILE_VAR), "{err}");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_missing_key_file_refuses_to_start() {
        let path = unique_path("missing");
        let err = resolve_from(
            Some("http://127.0.0.1:4310".into()),
            Some(path.to_string_lossy().into_owned()),
        )
        .unwrap_err();
        assert!(err.contains(KEY_FILE_VAR), "{err}");
        assert!(err.contains(&path.to_string_lossy().to_string()), "{err}");
    }

    #[test]
    fn a_key_file_that_is_a_directory_refuses_to_start() {
        let path = unique_path("is-a-dir");
        std::fs::create_dir_all(&path).expect("create dir");
        let err = resolve_from(
            Some("http://127.0.0.1:4310".into()),
            Some(path.to_string_lossy().into_owned()),
        )
        .unwrap_err();
        assert!(err.contains(KEY_FILE_VAR), "{err}");
        let _ = std::fs::remove_dir_all(&path);
    }

    #[test]
    fn a_key_file_that_is_empty_after_trimming_refuses_to_start() {
        let path = unique_path("empty");
        write(&path, "   \n\t  ");
        let err = resolve_from(
            Some("http://127.0.0.1:4310".into()),
            Some(path.to_string_lossy().into_owned()),
        )
        .unwrap_err();
        assert!(err.contains(KEY_FILE_VAR), "{err}");
        assert!(err.contains("empty"), "{err}");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_url_without_a_scheme_is_refused() {
        let path = unique_path("bad-url");
        write(&path, "key");
        let err = resolve_from(
            Some("127.0.0.1:4310".into()),
            Some(path.to_string_lossy().into_owned()),
        )
        .unwrap_err();
        assert!(err.contains(URL_VAR), "{err}");
        let _ = std::fs::remove_file(&path);
    }

    /// The key never appears in `Debug`, whatever the caller does with it -
    /// the same "assert the secret is absent from a Debug rendering" idiom
    /// `remote/wg_operator.rs`'s
    /// `a_dump_never_carries_the_interface_private_key_into_state` already
    /// uses for the WireGuard private key.
    #[test]
    fn the_key_never_appears_in_debug_output() {
        let path = unique_path("debug-redaction");
        write(&path, "top-secret-bytes-nobody-should-see");
        let resolved = resolve_from(
            Some("http://127.0.0.1:4310".into()),
            Some(path.to_string_lossy().into_owned()),
        )
        .expect("must resolve");
        let rendered = format!("{resolved:?}");
        assert!(
            !rendered.contains("top-secret-bytes-nobody-should-see"),
            "the key leaked into Debug: {rendered}"
        );
        assert!(rendered.contains("redacted"), "{rendered}");
        let _ = std::fs::remove_file(&path);
    }
}
