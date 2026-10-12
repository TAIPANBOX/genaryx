//! D8, the console's half of closing a retained reservation: the shared
//! secret, and the short-lived token minted from it.
//!
//! The gateway (TokenFuse `crates/gateway/src/closure.rs`) closes a retained
//! reservation only for a request carrying a token it can verify with a
//! secret it shares with this console. The console mints that token only
//! after a person confirmed the closure with a passkey assertion: the web
//! shell's ceremony gate runs before `money_close_retained` is dispatched at
//! all (`crates/web/src/main.rs`'s `SENSITIVE_COMMANDS`).
//!
//! **The token** is `v1.<id>.<revision>.<expiry>.<signature>`: the
//! reservation id in decimal, the revision as the 16 lowercase hex digits
//! `GET /v1/retained` lists, the expiry in Unix milliseconds, and the
//! signature the lowercase hex HMAC-SHA256, under the shared secret, of the
//! bytes `tokenfuse-closure-v1\n<id>\n<revision>\n<expiry>`. Byte for byte the
//! gateway's own `mint`; a test vector shared with it pins the format.
//!
//! **Lifetime** is [`TOKEN_TTL_MILLIS`], two minutes. The gateway refuses a
//! token that expires more than five minutes after it is presented
//! ([`MAX_TOKEN_TTL_MILLIS`]), and a compile-time check holds this console
//! under that bound.
//!
//! **Configuration**, resolved once at startup: `GENARYX_CLOSURE_SECRET_FILE`
//! names a file whose content, trimmed of surrounding whitespace, is the
//! secret (the same bytes the gateway reads from `TOKENFUSE_CLOSURE_SECRET`).
//! Unset is a normal box: `money_close_retained` answers "not configured" and
//! signs nothing. Set to a file that cannot be read, is empty, or holds fewer
//! than [`MIN_SECRET_BYTES`] bytes refuses to start, the same posture
//! `delegation::env` takes for its revoke key.
//!
//! This module lives in `genaryx-api` and nowhere else on purpose: Felyx
//! (`genaryx-copilot`) does not depend on this crate, so the copilot cannot
//! reach the secret or mint a token (held by
//! `the_copilot_crate_cannot_reach_the_closure_secret_or_mint_a_token`).

use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::path::Path;

/// The variable naming the secret file.
pub const SECRET_FILE_VAR: &str = "GENARYX_CLOSURE_SECRET_FILE";

/// The shortest secret accepted: 32 bytes, the HMAC-SHA256 output size, and
/// the gateway's own floor.
pub const MIN_SECRET_BYTES: usize = 32;

/// How long a minted token lives.
pub const TOKEN_TTL_MILLIS: i64 = 120_000;

/// The longest lifetime the gateway accepts, measured from when it is
/// presented.
pub const MAX_TOKEN_TTL_MILLIS: i64 = 300_000;

const _: () = assert!(TOKEN_TTL_MILLIS > 0 && TOKEN_TTL_MILLIS <= MAX_TOKEN_TTL_MILLIS);

const MAC_DOMAIN: &str = "tokenfuse-closure-v1";

/// The shared secret, wrapped so it cannot be printed by accident.
#[derive(Clone, PartialEq, Eq)]
pub struct ClosureSecret(Vec<u8>);

impl ClosureSecret {
    /// The secret's bytes. Its one caller is [`mint_token`] via
    /// `money::retained`.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

impl std::fmt::Debug for ClosureSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ClosureSecret(redacted)")
    }
}

/// Whether this console can mint closure tokens, resolved once at startup.
#[derive(Debug, Clone)]
pub enum ClosureConfig {
    /// `GENARYX_CLOSURE_SECRET_FILE` is unset: closing answers "not
    /// configured" and calls nobody.
    NotConfigured,
    Configured {
        secret: ClosureSecret,
    },
}

impl ClosureConfig {
    /// Resolve from the process environment, reading the file the one time
    /// this is called (startup).
    pub fn resolve_from_env() -> Result<Self, String> {
        Self::from_secret_file(std::env::var(SECRET_FILE_VAR).ok().as_deref())
    }

    /// Testable core of [`resolve_from_env`](Self::resolve_from_env): the
    /// variable's value, already read. A blank value counts as unset.
    pub fn from_secret_file(path: Option<&str>) -> Result<Self, String> {
        let Some(path) = path.map(str::trim).filter(|p| !p.is_empty()) else {
            return Ok(ClosureConfig::NotConfigured);
        };
        let raw = std::fs::read(Path::new(path))
            .map_err(|e| format!("{SECRET_FILE_VAR} ({path}) cannot be read: {e}"))?;
        let secret = trim_ascii_whitespace(&raw);
        if secret.is_empty() {
            return Err(format!("{SECRET_FILE_VAR} ({path}) is empty"));
        }
        if secret.len() < MIN_SECRET_BYTES {
            return Err(format!(
                "{SECRET_FILE_VAR} ({path}) holds {} bytes; the closure secret must be at least \
                 {MIN_SECRET_BYTES} bytes, or unset the variable to turn closing off",
                secret.len()
            ));
        }
        Ok(ClosureConfig::Configured {
            secret: ClosureSecret(secret.to_vec()),
        })
    }
}

fn trim_ascii_whitespace(bytes: &[u8]) -> &[u8] {
    let start = bytes
        .iter()
        .position(|b| !b.is_ascii_whitespace())
        .unwrap_or(bytes.len());
    let end = bytes
        .iter()
        .rposition(|b| !b.is_ascii_whitespace())
        .map_or(start, |i| i + 1);
    &bytes[start..end]
}

/// Mint a closure token for reservation `id` at `revision`, expiring at
/// `expiry_millis` (Unix milliseconds). See the module doc for the format.
pub fn mint_token(secret: &[u8], id: u64, revision: &str, expiry_millis: i64) -> String {
    let mut mac =
        Hmac::<Sha256>::new_from_slice(secret).expect("HMAC-SHA256 accepts a key of any length");
    mac.update(format!("{MAC_DOMAIN}\n{id}\n{revision}\n{expiry_millis}").as_bytes());
    let sig: String = mac
        .finalize()
        .into_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    format!("v1.{id}.{revision}.{expiry_millis}.{sig}")
}

/// The expiry for a token minted at `now_millis`.
pub fn expiry_for(now_millis: i64) -> i64 {
    now_millis.saturating_add(TOKEN_TTL_MILLIS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_message_is_the_domain_then_id_revision_expiry_on_their_own_lines() {
        // Recomputed with the hmac crate directly over the literal bytes, so
        // a change to the message layout cannot hide behind mint_token.
        let mut mac = Hmac::<Sha256>::new_from_slice(b"k").unwrap();
        mac.update(b"tokenfuse-closure-v1\n1\naaaaaaaaaaaaaaaa\n2");
        let want: String = mac
            .finalize()
            .into_bytes()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(
            mint_token(b"k", 1, "aaaaaaaaaaaaaaaa", 2),
            format!("v1.1.aaaaaaaaaaaaaaaa.2.{want}")
        );
    }

    #[test]
    fn expiry_is_two_minutes_out_and_saturates() {
        assert_eq!(expiry_for(1_000), 121_000);
        assert_eq!(expiry_for(i64::MAX), i64::MAX);
    }

    #[test]
    fn surrounding_whitespace_is_trimmed_and_inner_bytes_kept() {
        assert_eq!(trim_ascii_whitespace(b"  a b\n"), b"a b");
        assert_eq!(trim_ascii_whitespace(b"\n\t "), b"");
        assert_eq!(trim_ascii_whitespace(b""), b"");
    }

    /// 200 seeds of ids, revisions and expiries: every token has the five
    /// parts the gateway's `verify` splits on, a 64-digit lowercase hex
    /// signature, and stays under the gateway's 256-byte token cap.
    #[test]
    fn two_hundred_seeds_of_tokens_keep_the_gateway_shape() {
        let mut x: u64 = 0x9e37_79b9_7f4a_7c15;
        for _ in 0..200 {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            let id = x;
            let rev = format!("{:016x}", x.rotate_left(17));
            let exp = (x >> 1) as i64;
            let t = mint_token(b"0123456789abcdef0123456789abcdef", id, &rev, exp);
            let parts: Vec<&str> = t.split('.').collect();
            assert_eq!(parts.len(), 5, "{t}");
            assert_eq!(parts[1], id.to_string());
            assert_eq!(parts[2], rev);
            assert_eq!(parts[3], exp.to_string());
            assert_eq!(parts[4].len(), 64);
            assert!(
                parts[4]
                    .bytes()
                    .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
            );
            assert!(t.len() <= 256, "{}", t.len());
        }
    }
}
