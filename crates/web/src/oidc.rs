//! Offline-by-default OIDC / JWT bearer verification for the console's IdP
//! login (docs/CONSOLE-IDP.md, B3/1), with an optional LIVE key source so
//! sign-in survives the IdP rotating its signing keys (CLAUDE.md invariant
//! 11).
//!
//! A conservative, default-off alternative to the local Argon2id account: the
//! customer hands the box a JWKS from their IdP and an operator signs in with
//! an OIDC ID-token instead of a password. The verification rules mirror,
//! almost line for line, tokenfuse-cloud's own `crates/cloud/src/oidc.rs` -
//! the same crate (`jsonwebtoken`), the same alg-confusion defense - because
//! that module was already reviewed and is the house pattern for "verify an
//! enterprise JWT". tokenfuse-cloud's own path stays static-only; the live
//! fetch below is this console's own addition.
//!
//! * **Offline by default, live if the operator asks.**
//!   `GENARYX_WEB_OIDC_JWKS` (inline JSON or a file path) is the original,
//!   still-default path: no network fetch ever, air-gap safe.
//!   `GENARYX_WEB_OIDC_JWKS_URL` (an `https://` URL) is the alternative: the
//!   box fetches and caches the JWKS itself, so a key rotation at the IdP does
//!   not lock every operator out until someone edits configuration. Setting
//!   BOTH, or setting the URL to anything but `https://`, refuses to start
//!   (exit non-zero): two sources of truth, or a key fetch nobody can trust,
//!   are not degraded modes worth booting into.
//! * **What is fetched, when, and what is refused.** A `GET` with a 5 s
//!   timeout, no redirect ever followed, the body read incrementally and
//!   capped at 1 MiB, 2xx required (see [`ReqwestFetcher`]). A fetch happens
//!   at startup (once, best-effort - a failed startup fetch does not stop the
//!   console, which starts with an empty live set and the local Argon2id
//!   account still reachable), before a verification if the current set is
//!   older than one hour (`MAX_AGE`), and on an unknown `kid` at most once
//!   every five minutes (`COOLDOWN`). Concurrent sign-ins never start two
//!   fetches: a `tokio::sync::Mutex` is held across the whole decide-then-fetch
//!   step. A fetched body is refused, and the last good set stays in force,
//!   unless it is valid JSON with a non-empty `keys` array, every key is RSA
//!   or EC, and no key carries a private-key member (`d`, `p`, `q`, `dp`,
//!   `dq`, `qi`, `k`) - an IdP publishing a private key is a compromise to
//!   refuse loudly, not a key to use (see [`validate_jwks_bytes`]).
//! * **Default off.** [`OidcConfig::from_env`] returns `None` unless issuer,
//!   audience and a JWKS source are all configured. When `None`, the login
//!   route never calls in here, so a password-only box is byte-for-byte
//!   unchanged.
//! * **Local account always wins as break-glass.** OIDC is additive; the
//!   Argon2id owner account is never removed by turning OIDC on.
//! * **Least privilege.** A verified token is a [`Role::Viewer`] unless the
//!   roles claim explicitly names the approver or admin role.
//!
//! ## What is validated (any failure => token rejected)
//!
//! 1. Well-formed JWS with a `kid` header.
//! 2. `kid` matches a key in the current JWKS - static, or the live cache
//!    (see [`KeySource`]), refreshed first per the rules above.
//! 3. Signature verifies with algorithms derived from the JWK KEY TYPE, never
//!    the attacker-controlled token header (closes RS256->HS256 alg confusion).
//! 4. `exp`, `iss`, `aud` are present (`set_required_spec_claims`) and valid.
//! 5. `sub` is present and non-empty (it names the human for the audit trail).
//!
//! These five do not fork between the two sources: [`verify`] resolves the
//! key first (a plain lookup, or an async cache read that may refresh), then
//! runs the same checks either way.

use crate::roles::Role;
use async_trait::async_trait;
use futures_util::StreamExt;
use jsonwebtoken::jwk::{AlgorithmParameters, Jwk, JwkSet};
use jsonwebtoken::{Algorithm, DecodingKey, Validation, decode, decode_header};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;
use std::time::{Duration, Instant};

const DEFAULT_SUB_CLAIM: &str = "sub";
const DEFAULT_ROLES_CLAIM: &str = "roles";
const DEFAULT_ADMIN_ROLE: &str = "genaryx-admin";
const DEFAULT_APPROVER_ROLE: &str = "genaryx-approver";

/// How long a fetched key set is trusted before a verification refreshes it
/// FIRST, unconditionally (module doc). Fixed rather than configurable: this
/// and the other two numbers below are the same kind of decision, and a
/// per-box knob would just move the argument about "how stale is too stale"
/// onto every operator instead of settling it once.
const MAX_AGE: Duration = Duration::from_secs(60 * 60);
/// The floor between two fetches triggered by an unknown `kid` ALONE, so a
/// flood of tokens carrying invented key ids costs the IdP at most one
/// request every five minutes.
const COOLDOWN: Duration = Duration::from_secs(5 * 60);
/// The real fetcher's own timeout (module doc step 2).
const FETCH_TIMEOUT: Duration = Duration::from_secs(5);
/// The real fetcher's own body cap, read incrementally (module doc step 2).
const MAX_JWKS_BODY_BYTES: usize = 1024 * 1024;
/// A JWK member that only ever belongs on a PRIVATE key. Any key in a fetched
/// set carrying one of these refuses the whole set (module doc step 3).
const FORBIDDEN_PRIVATE_MEMBERS: &[&str] = &["d", "p", "q", "dp", "dq", "qi", "k"];

/// Static, offline OIDC config, built once at startup and held on the app
/// state. No env or file I/O happens per request; a live key source (below)
/// does its own I/O, but only ever through [`verify`]'s async path.
#[derive(Debug, Clone)]
pub struct OidcConfig {
    issuer: String,
    audience: String,
    keys: KeySource,
    sub_claim: String,
    roles_claim: String,
    admin_role: String,
    approver_role: String,
}

/// A verified token: the human's console username, mapped role, and the audit
/// actor id. The raw token never leaves this module (it is a bearer secret).
pub struct Verified {
    /// The `sub` claim: the username shown in the UI and stored on the session.
    pub username: String,
    pub role: Role,
}

impl OidcConfig {
    /// Build from explicit parts, parsing `jwks_json`: the STATIC source,
    /// byte-for-byte the same behaviour this had before the live source
    /// existed. `None` (OIDC disabled) if issuer/audience is empty or the
    /// JWKS is missing/empty/unparseable - a misconfiguration fails SAFE (no
    /// token is ever accepted). Exposed so tests can build a config around an
    /// in-test signing key.
    pub fn new(
        issuer: impl Into<String>,
        audience: impl Into<String>,
        jwks_json: &str,
        sub_claim: impl Into<String>,
        roles_claim: impl Into<String>,
        admin_role: impl Into<String>,
        approver_role: impl Into<String>,
    ) -> Option<OidcConfig> {
        let issuer = issuer.into();
        let audience = audience.into();
        if issuer.is_empty() || audience.is_empty() {
            return None;
        }
        let jwks: JwkSet = serde_json::from_str(jwks_json).ok()?;
        if jwks.keys.is_empty() {
            return None;
        }
        Some(Self::finish(
            issuer,
            audience,
            KeySource::Static(jwks),
            sub_claim,
            roles_claim,
            admin_role,
            approver_role,
        ))
    }

    fn finish(
        issuer: String,
        audience: String,
        keys: KeySource,
        sub_claim: impl Into<String>,
        roles_claim: impl Into<String>,
        admin_role: impl Into<String>,
        approver_role: impl Into<String>,
    ) -> OidcConfig {
        OidcConfig {
            issuer,
            audience,
            keys,
            sub_claim: non_empty(sub_claim.into(), DEFAULT_SUB_CLAIM),
            roles_claim: non_empty(roles_claim.into(), DEFAULT_ROLES_CLAIM),
            admin_role: non_empty(admin_role.into(), DEFAULT_ADMIN_ROLE),
            approver_role: non_empty(approver_role.into(), DEFAULT_APPROVER_ROLE),
        }
    }

    /// Build from the environment, or `None` when OIDC is unconfigured.
    ///
    /// Required - absent => OIDC disabled: `GENARYX_WEB_OIDC_ISSUER`,
    /// `GENARYX_WEB_OIDC_AUDIENCE`, and exactly one of `GENARYX_WEB_OIDC_JWKS`
    /// (inline JSON, or a path to a file holding it) or
    /// `GENARYX_WEB_OIDC_JWKS_URL` (an `https://` URL, module doc). Setting
    /// both JWKS variables, or a URL that is not `https://`, REFUSES TO START
    /// (exit non-zero) rather than guessing which the operator meant.
    /// Optional: `GENARYX_WEB_OIDC_SUB_CLAIM` (default `sub`),
    /// `GENARYX_WEB_OIDC_ROLES_CLAIM` (default `roles`),
    /// `GENARYX_WEB_OIDC_ADMIN_ROLE` (default `genaryx-admin`),
    /// `GENARYX_WEB_OIDC_APPROVER_ROLE` (default `genaryx-approver`).
    pub fn from_env() -> Option<OidcConfig> {
        let issuer = env_nonempty("GENARYX_WEB_OIDC_ISSUER")?;
        let audience = env_nonempty("GENARYX_WEB_OIDC_AUDIENCE")?;
        let sub_claim = std::env::var("GENARYX_WEB_OIDC_SUB_CLAIM").unwrap_or_default();
        let roles_claim = std::env::var("GENARYX_WEB_OIDC_ROLES_CLAIM").unwrap_or_default();
        let admin_role = std::env::var("GENARYX_WEB_OIDC_ADMIN_ROLE").unwrap_or_default();
        let approver_role = std::env::var("GENARYX_WEB_OIDC_APPROVER_ROLE").unwrap_or_default();

        match resolve_jwks_source(
            env_nonempty("GENARYX_WEB_OIDC_JWKS"),
            env_nonempty("GENARYX_WEB_OIDC_JWKS_URL"),
        ) {
            Ok(KeySourceChoice::Unconfigured) => None,
            // The static path: unchanged from before the live source existed,
            // down to reusing `new` itself rather than a second copy of its
            // parse-and-validate logic.
            Ok(KeySourceChoice::Static(raw)) => {
                let jwks_json = load_jwks(&raw)?;
                OidcConfig::new(
                    issuer,
                    audience,
                    &jwks_json,
                    sub_claim,
                    roles_claim,
                    admin_role,
                    approver_role,
                )
            }
            Ok(KeySourceChoice::LiveUrl(url)) => Some(Self::finish(
                issuer,
                audience,
                KeySource::Live(Arc::new(LiveJwks::new(
                    url,
                    Arc::new(ReqwestFetcher),
                    Arc::new(SystemClock),
                ))),
                sub_claim,
                roles_claim,
                admin_role,
                approver_role,
            )),
            Err(refusal) => {
                eprintln!("genaryx-web: {refusal}");
                std::process::exit(1);
            }
        }
    }

    /// Kick the live source's one startup fetch (module doc); a no-op for the
    /// static source and for a live source that has already fetched once.
    /// Called from [`crate::ctx::Ctx::resolve`], spawned in the background so
    /// a slow or unreachable IdP delays nothing else the console serves.
    pub(crate) async fn warm_up(&self) {
        if let KeySource::Live(live) = &self.keys {
            live.warm_up().await;
        }
    }
}

#[derive(Deserialize)]
struct Claims {
    #[serde(flatten)]
    extra: HashMap<String, serde_json::Value>,
}

impl Claims {
    fn string(&self, key: &str) -> Option<String> {
        match self.extra.get(key)? {
            serde_json::Value::String(s) if !s.is_empty() => Some(s.clone()),
            _ => None,
        }
    }

    /// Whether the roles claim contains `role`. Accepts a JSON array of
    /// strings or a single space-separated string - the two shapes IdPs emit.
    fn has_role(&self, key: &str, role: &str) -> bool {
        match self.extra.get(key) {
            Some(serde_json::Value::Array(items)) => items.iter().any(|v| v.as_str() == Some(role)),
            Some(serde_json::Value::String(s)) => s.split_whitespace().any(|r| r == role),
            _ => false,
        }
    }
}

/// Verify an OIDC ID-token and map it to a username + role, or `None` on any
/// validation failure. See the module docs for the exact checks. Role is
/// `admin` if the roles claim contains the admin role, else `approver` if it
/// contains the approver role, else `viewer` (least privilege).
pub async fn verify(cfg: &OidcConfig, token: &str) -> Option<Verified> {
    // 1. Well-formed header with a key id.
    let header = decode_header(token).ok()?;
    let kid = header.kid?;

    // 2. Key id matches a key in the current JWKS - static, or the live cache
    //    refreshed first per the rules in the module doc.
    let jwk = cfg.keys.find(&kid).await?;

    // 3. Allowed algorithms come from the key TYPE, never the token header -
    //    prevents an attacker downgrading an RSA/EC key to HS256 and forging a
    //    signature ("alg confusion"). Symmetric / OKP keys are rejected.
    let algorithms: Vec<Algorithm> = match &jwk.algorithm {
        AlgorithmParameters::RSA(_) => vec![Algorithm::RS256, Algorithm::RS384, Algorithm::RS512],
        AlgorithmParameters::EllipticCurve(_) => vec![Algorithm::ES256, Algorithm::ES384],
        _ => return None,
    };
    let key = DecodingKey::from_jwk(&jwk).ok()?;

    // 4. Signature + exp + iss + aud. `set_required_spec_claims` makes a token
    //    that OMITS exp/iss/aud a rejection, not a pass (jsonwebtoken only
    //    checks iss/aud when present by default - an audience-confusion risk
    //    if an IdP reuses a signing key across services).
    let mut validation = Validation::new(algorithms[0]);
    validation.algorithms = algorithms;
    validation.validate_exp = true;
    validation.set_required_spec_claims(&["exp", "iss", "aud"]);
    validation.set_issuer(&[&cfg.issuer]);
    validation.set_audience(&[&cfg.audience]);
    let data = decode::<Claims>(token, &key, &validation).ok()?;
    let claims = data.claims;

    // 5. `sub` is mandatory - it names the human for the audit trail.
    let username = claims.string(&cfg.sub_claim)?;

    let role = if claims.has_role(&cfg.roles_claim, &cfg.admin_role) {
        Role::Admin
    } else if claims.has_role(&cfg.roles_claim, &cfg.approver_role) {
        Role::Approver
    } else {
        Role::Viewer
    };

    Some(Verified { username, role })
}

// ---------------------------------------------------------------------------
// key source: static (unchanged) or live (fetched and cached)
// ---------------------------------------------------------------------------

/// Where the current signing keys come from.
#[derive(Debug, Clone)]
enum KeySource {
    /// Today's behaviour, byte for byte: whatever `OidcConfig::new` parsed
    /// once at construction. Never refetched.
    Static(JwkSet),
    /// Fetched over HTTPS and cached (module doc); shared via `Arc` because
    /// [`OidcConfig`] itself is `Clone` (the warm-up spawn in `ctx::resolve`
    /// needs its own owned handle onto the SAME cache the request path
    /// reads).
    Live(Arc<LiveJwks>),
}

impl KeySource {
    async fn find(&self, kid: &str) -> Option<Jwk> {
        match self {
            KeySource::Static(set) => set.find(kid).cloned(),
            KeySource::Live(live) => live.find(kid).await,
        }
    }
}

/// The live JWKS cache: the current set, when it was last fetched
/// successfully, and when it was last ATTEMPTED - all behind one lock so a
/// decide-then-fetch step is atomic (module doc: single-flight).
struct LiveJwks {
    url: String,
    fetcher: Arc<dyn JwksFetcher>,
    clock: Arc<dyn Clock>,
    state: tokio::sync::Mutex<LiveState>,
}

struct LiveState {
    keys: JwkSet,
    /// When `keys` was last REPLACED by a successful fetch. `None` means
    /// never - the box just started and the live set is still empty.
    fetched_at: Option<Instant>,
    /// When a fetch was last ATTEMPTED, success or failure - what the
    /// cooldown between kid-miss fetches measures from.
    last_attempt: Option<Instant>,
    /// Whether the CURRENT run of consecutive failures has already logged a
    /// warning. Reset on the next success, so a multi-hour outage against a
    /// box that keeps retrying every cooldown/hour logs once, not on every
    /// attempt (module doc: "once per failure streak, not per request").
    warned_this_streak: bool,
}

impl LiveState {
    fn empty() -> Self {
        Self {
            keys: JwkSet { keys: Vec::new() },
            fetched_at: None,
            last_attempt: None,
            warned_this_streak: false,
        }
    }
}

impl fmt::Debug for LiveJwks {
    // Manual: `dyn JwksFetcher`/`dyn Clock` need not be `Debug` themselves,
    // and the URL is the one field worth a reader's attention.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LiveJwks")
            .field("url", &self.url)
            .finish_non_exhaustive()
    }
}

impl LiveJwks {
    fn new(url: String, fetcher: Arc<dyn JwksFetcher>, clock: Arc<dyn Clock>) -> Self {
        Self {
            url,
            fetcher,
            clock,
            state: tokio::sync::Mutex::new(LiveState::empty()),
        }
    }

    /// The one startup fetch (module doc step 4a): only when nothing has
    /// been fetched yet, best-effort, never returns an error - a failure here
    /// is exactly what the staleness/kid-miss rules below exist to retry.
    async fn warm_up(&self) {
        let mut state = self.state.lock().await;
        if state.fetched_at.is_none() {
            self.attempt_locked(&mut state).await;
        }
    }

    /// Resolve `kid` against the current set, refreshing first per the
    /// module doc's rules (steps 4b/4c). The lock is held for the ENTIRE
    /// call, including any fetch inside it, which is what makes concurrent
    /// callers single-flight (step 4d) rather than each starting their own
    /// fetch: a second caller blocks here until the first's attempt (and its
    /// state update) is finished, then makes its own decision from the fresh
    /// state rather than the stale one it started with.
    async fn find(&self, kid: &str) -> Option<Jwk> {
        let mut state = self.state.lock().await;
        let now = self.clock.now();

        let stale = match state.fetched_at {
            None => true,
            Some(t) => now.saturating_duration_since(t) >= MAX_AGE,
        };
        if stale {
            self.attempt_locked(&mut state).await;
        } else if state.keys.find(kid).is_none() {
            let cooled_down = match state.last_attempt {
                None => true,
                Some(t) => now.saturating_duration_since(t) >= COOLDOWN,
            };
            if cooled_down {
                self.attempt_locked(&mut state).await;
            }
        }
        state.keys.find(kid).cloned()
    }

    /// One fetch attempt, called with the lock already held. Any failure -
    /// transport, or a hostile body `validate_jwks_bytes` refuses - leaves
    /// `state.keys` exactly as it was; only a validated success replaces it.
    /// `last_attempt` moves to `now` either way, which is what the cooldown
    /// in [`Self::find`] measures from.
    async fn attempt_locked(&self, state: &mut LiveState) {
        let now = self.clock.now();
        state.last_attempt = Some(now);
        let outcome = match self.fetcher.fetch(&self.url).await {
            Ok(body) => validate_jwks_bytes(&body).map_err(|e| e.to_string()),
            Err(e) => Err(e.to_string()),
        };
        match outcome {
            Ok(keys) => {
                state.keys = keys;
                state.fetched_at = Some(now);
                state.warned_this_streak = false;
            }
            Err(reason) => {
                if !state.warned_this_streak {
                    warn_fetch_failed(&self.url, &reason);
                    state.warned_this_streak = true;
                }
            }
        }
    }
}

fn warn_fetch_failed(url: &str, reason: &str) {
    tracing::warn!(
        url = %url,
        error = %reason,
        "oidc: live JWKS fetch failed; keeping the current key set (empty until the first \
         success) and retrying per the module's refresh rules"
    );
}

/// An injectable clock, so a test can fast-forward past `MAX_AGE`/`COOLDOWN`
/// without an actual wait. [`SystemClock`] is the only production
/// implementation.
trait Clock: Send + Sync {
    fn now(&self) -> Instant;
}

#[derive(Debug, Clone, Default)]
struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

// ---------------------------------------------------------------------------
// transport seam
// ---------------------------------------------------------------------------

/// What a JWKS fetch attempt can fail with. Variants rather than only a
/// string so a test can assert exactly which rule fired.
#[derive(Debug, Clone, PartialEq, Eq)]
enum FetchError {
    Transport(String),
    NonSuccess(u16),
    Redirected,
    TooLarge,
}

impl fmt::Display for FetchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FetchError::Transport(e) => write!(f, "transport error: {e}"),
            FetchError::NonSuccess(code) => write!(f, "http status {code}"),
            FetchError::Redirected => {
                write!(f, "the server tried to redirect a JWKS fetch; refused")
            }
            FetchError::TooLarge => {
                write!(f, "body exceeded {MAX_JWKS_BODY_BYTES} bytes; refused")
            }
        }
    }
}

/// The transport seam (module doc): [`LiveJwks`] never touches `reqwest`
/// directly, so a test injects a fake and asserts exactly how many times it
/// was called - a real socket would make "one fetch per cooldown" and
/// "single-flight" both timing-flaky. [`ReqwestFetcher`] is the only
/// production implementation.
#[async_trait]
trait JwksFetcher: Send + Sync {
    async fn fetch(&self, url: &str) -> Result<Vec<u8>, FetchError>;
}

/// The real fetcher: a `GET` with a hard timeout, no redirect ever followed
/// (a redirect is treated as a fetch failure, not taken), and the body read
/// incrementally so a hostile response cannot allocate past the cap before
/// this notices.
#[derive(Debug, Clone, Default)]
struct ReqwestFetcher;

#[async_trait]
impl JwksFetcher for ReqwestFetcher {
    async fn fetch(&self, url: &str) -> Result<Vec<u8>, FetchError> {
        let client = reqwest::Client::builder()
            .timeout(FETCH_TIMEOUT)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| FetchError::Transport(e.to_string()))?;
        let resp = client
            .get(url)
            .send()
            .await
            .map_err(|e| FetchError::Transport(e.to_string()))?;
        if resp.status().is_redirection() {
            return Err(FetchError::Redirected);
        }
        if !resp.status().is_success() {
            return Err(FetchError::NonSuccess(resp.status().as_u16()));
        }
        let mut body = Vec::new();
        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|e| FetchError::Transport(e.to_string()))?;
            if body.len() + chunk.len() > MAX_JWKS_BODY_BYTES {
                return Err(FetchError::TooLarge);
            }
            body.extend_from_slice(&chunk);
        }
        Ok(body)
    }
}

// ---------------------------------------------------------------------------
// hostile-set validation
// ---------------------------------------------------------------------------

/// Why a fetched JWKS body is refused (module doc step 3). Kept as data
/// rather than only a log line so tests can assert exactly which rule fired.
#[derive(Debug, Clone, PartialEq, Eq)]
enum RefusedSet {
    InvalidJson,
    NotAnObject,
    KeysMissingOrNotArray,
    EmptyKeys,
    KeyNotAnObject,
    UnsupportedKeyType(String),
    PrivateKeyMember {
        kid: Option<String>,
        member: &'static str,
    },
}

impl fmt::Display for RefusedSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RefusedSet::InvalidJson => write!(f, "not valid JSON"),
            RefusedSet::NotAnObject => write!(f, "the top level is not a JSON object"),
            RefusedSet::KeysMissingOrNotArray => write!(f, "\"keys\" is missing or not an array"),
            RefusedSet::EmptyKeys => write!(f, "\"keys\" is empty"),
            RefusedSet::KeyNotAnObject => write!(f, "a key entry is not a JSON object"),
            RefusedSet::UnsupportedKeyType(kty) => {
                write!(f, "key type {kty:?} is neither RSA nor EC")
            }
            RefusedSet::PrivateKeyMember { kid, member } => write!(
                f,
                "key {} carries the private-key member {member:?}",
                kid.as_deref().unwrap_or("<no kid>")
            ),
        }
    }
}

/// Validate a fetched JWKS body against the rules in the module doc (step 3),
/// or say which rule refused it. Runs on the RAW `serde_json::Value` BEFORE
/// any typed parse into [`JwkSet`]: `jsonwebtoken`'s own
/// `RSAKeyParameters`/`EllipticCurveKeyParameters` model only the PUBLIC
/// members (`n`/`e`, `x`/`y`) and have no `deny_unknown_fields`, so they
/// silently DROP anything else on parse - a `d` the source JSON carried would
/// already be gone, and unrecoverable, by the time a `JwkSet` exists. This
/// function is the only point a leaked private key is still visible.
fn validate_jwks_bytes(body: &[u8]) -> Result<JwkSet, RefusedSet> {
    let raw: Value = serde_json::from_slice(body).map_err(|_| RefusedSet::InvalidJson)?;
    let obj = raw.as_object().ok_or(RefusedSet::NotAnObject)?;
    let keys = obj
        .get("keys")
        .and_then(Value::as_array)
        .ok_or(RefusedSet::KeysMissingOrNotArray)?;
    if keys.is_empty() {
        return Err(RefusedSet::EmptyKeys);
    }
    for key in keys {
        let key_obj = key.as_object().ok_or(RefusedSet::KeyNotAnObject)?;
        let kty = key_obj.get("kty").and_then(Value::as_str).unwrap_or("");
        if kty != "RSA" && kty != "EC" {
            return Err(RefusedSet::UnsupportedKeyType(kty.to_string()));
        }
        for member in FORBIDDEN_PRIVATE_MEMBERS {
            if key_obj.contains_key(*member) {
                let kid = key_obj
                    .get("kid")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                return Err(RefusedSet::PrivateKeyMember { kid, member });
            }
        }
    }
    // Only now the typed parse, which is what `find` actually uses. Cannot
    // fail given the checks above already confirmed an object with an array
    // of key objects, but `?` (via `map_err`) over `unwrap` keeps this
    // function panic-free by construction rather than by review.
    serde_json::from_value(raw).map_err(|_| RefusedSet::InvalidJson)
}

// ---------------------------------------------------------------------------
// startup configuration: refuse to start, or say which source to use
// ---------------------------------------------------------------------------

/// Why `from_env` refuses to let the process start at all, distinct from its
/// ordinary default-off return of `None` for a merely ABSENT config. Both
/// cases here are a config an operator asked for that cannot be honoured
/// safely, so guessing which half they meant would be worse than refusing
/// loudly at boot, before any browser ever reaches this box.
#[derive(Debug, Clone, PartialEq, Eq)]
enum StartupRefusal {
    BothJwksSourcesSet,
    JwksUrlNotHttps(String),
}

impl fmt::Display for StartupRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StartupRefusal::BothJwksSourcesSet => write!(
                f,
                "both GENARYX_WEB_OIDC_JWKS and GENARYX_WEB_OIDC_JWKS_URL are set; genaryx-web \
                 needs exactly one source of truth for the JWKS. Unset one and restart."
            ),
            StartupRefusal::JwksUrlNotHttps(url) => write!(
                f,
                "GENARYX_WEB_OIDC_JWKS_URL must be an https:// URL, got {url:?}. genaryx-web \
                 refuses to fetch signing keys over a connection it cannot trust."
            ),
        }
    }
}

#[derive(Debug)]
enum KeySourceChoice {
    /// Neither JWKS variable is set: OIDC stays off, `from_env`'s ordinary
    /// (and unchanged) default-off behaviour.
    Unconfigured,
    /// `GENARYX_WEB_OIDC_JWKS`'s raw value (inline JSON or a file path).
    Static(String),
    /// `GENARYX_WEB_OIDC_JWKS_URL`, already confirmed `https://`.
    LiveUrl(String),
}

/// The pure decision behind `from_env`'s two "refuse to start" cases: no
/// process exit and no I/O happen here, which is what makes both directly
/// testable. `jwks_static`/`jwks_url` are the two env readings, already
/// `None` when unset or blank.
fn resolve_jwks_source(
    jwks_static: Option<String>,
    jwks_url: Option<String>,
) -> Result<KeySourceChoice, StartupRefusal> {
    match (jwks_static, jwks_url) {
        (Some(_), Some(_)) => Err(StartupRefusal::BothJwksSourcesSet),
        (None, None) => Ok(KeySourceChoice::Unconfigured),
        (Some(raw), None) => Ok(KeySourceChoice::Static(raw)),
        (None, Some(url)) => {
            if url.starts_with("https://") {
                Ok(KeySourceChoice::LiveUrl(url))
            } else {
                Err(StartupRefusal::JwksUrlNotHttps(url))
            }
        }
    }
}

fn non_empty(value: String, default: &str) -> String {
    if value.trim().is_empty() {
        default.to_string()
    } else {
        value
    }
}

fn env_nonempty(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.trim().is_empty())
}

/// `raw` is either the JWKS JSON itself or a path to a file holding it. A
/// value that starts with `{` is treated as inline JSON; anything else is
/// read as a file path. Never a URL: the static path is never fetched.
fn load_jwks(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.starts_with('{') {
        Some(trimmed.to_string())
    } else {
        std::fs::read_to_string(trimmed).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{EncodingKey, Header, encode};
    use std::collections::VecDeque;
    use std::sync::Mutex as StdMutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    // A fixed ES256 test keypair (PKCS#8 PEM private + the matching public JWK
    // with kid "test-key"), generated once with openssl for tests; never a
    // real key. The JWK x/y are the base64url P-256 coords of PRIV_PEM.
    const PRIV_PEM: &str = "-----BEGIN PRIVATE KEY-----\nMIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgSF30BcU02a19uA3P\n1YcMrDkSfQWiKtjy4jLGGykVcHOhRANCAASCUZm+yqkv9wBNDdveC0nYLscslVCb\nFPNbeKd6A+DgxTZdKiFLdC1NbkWHNPq8FzEyh/aiC356Mqz7iF1L42Ve\n-----END PRIVATE KEY-----\n";
    // Matching public key as a JWK set (x/y are the base64url EC coords).
    const JWKS: &str = r#"{"keys":[{"kty":"EC","crv":"P-256","kid":"test-key","x":"glGZvsqpL_cATQ3b3gtJ2C7HLJVQmxTzW3inegPg4MU","y":"Nl0qIUt0LU1uRYc0-rwXMTKH9qILfnoyrPuIXUvjZV4"}]}"#;

    fn cfg() -> OidcConfig {
        OidcConfig::new(
            "https://idp.example",
            "genaryx-console",
            JWKS,
            "",
            "",
            "",
            "",
        )
        .expect("valid config")
    }

    fn token_with(claims: serde_json::Value) -> String {
        let mut header = Header::new(Algorithm::ES256);
        header.kid = Some("test-key".to_string());
        let key = EncodingKey::from_ec_pem(PRIV_PEM.as_bytes()).expect("valid key");
        encode(&header, &claims, &key).expect("encode")
    }

    fn future() -> i64 {
        // A fixed far-future exp so the test never depends on the clock beyond
        // "not expired". jsonwebtoken compares against the real now(), so this
        // must be an absolute timestamp well ahead.
        4_102_444_800 // 2100-01-01
    }

    #[tokio::test]
    async fn a_valid_admin_token_verifies_and_maps_admin() {
        let tok = token_with(serde_json::json!({
            "iss": "https://idp.example",
            "aud": "genaryx-console",
            "sub": "alice",
            "roles": ["genaryx-admin", "something-else"],
            "exp": future(),
        }));
        let v = verify(&cfg(), &tok).await.expect("verifies");
        assert_eq!(v.username, "alice");
        assert_eq!(v.role, Role::Admin);
    }

    #[tokio::test]
    async fn approver_and_viewer_roles_map_least_privilege() {
        let approver = token_with(serde_json::json!({
            "iss": "https://idp.example", "aud": "genaryx-console",
            "sub": "bob", "roles": "genaryx-approver", "exp": future(),
        }));
        assert_eq!(
            verify(&cfg(), &approver).await.unwrap().role,
            Role::Approver
        );

        // No known role => viewer, never a default promotion.
        let viewer = token_with(serde_json::json!({
            "iss": "https://idp.example", "aud": "genaryx-console",
            "sub": "carol", "roles": ["unrelated"], "exp": future(),
        }));
        assert_eq!(verify(&cfg(), &viewer).await.unwrap().role, Role::Viewer);
    }

    #[tokio::test]
    async fn wrong_issuer_audience_or_missing_sub_are_rejected() {
        for bad in [
            serde_json::json!({"iss":"https://evil","aud":"genaryx-console","sub":"x","exp":future()}),
            serde_json::json!({"iss":"https://idp.example","aud":"other","sub":"x","exp":future()}),
            serde_json::json!({"iss":"https://idp.example","aud":"genaryx-console","exp":future()}),
            // Expired.
            serde_json::json!({"iss":"https://idp.example","aud":"genaryx-console","sub":"x","exp":1}),
        ] {
            assert!(verify(&cfg(), &token_with(bad)).await.is_none());
        }
    }

    #[tokio::test]
    async fn a_token_signed_by_an_unknown_key_is_rejected() {
        // Re-sign with a different kid the JWKS does not contain.
        let mut header = Header::new(Algorithm::ES256);
        header.kid = Some("not-in-jwks".to_string());
        let key = EncodingKey::from_ec_pem(PRIV_PEM.as_bytes()).unwrap();
        let tok = encode(
            &header,
            &serde_json::json!({
                "iss":"https://idp.example","aud":"genaryx-console","sub":"x","exp":future()
            }),
            &key,
        )
        .unwrap();
        assert!(verify(&cfg(), &tok).await.is_none());
    }

    #[test]
    fn disabled_config_when_parts_missing() {
        assert!(OidcConfig::new("", "aud", JWKS, "", "", "", "").is_none());
        assert!(OidcConfig::new("iss", "aud", "{}", "", "", "", "").is_none());
        assert!(OidcConfig::new("iss", "aud", "not json", "", "", "", "").is_none());
    }

    // -----------------------------------------------------------------
    // live source: fixtures
    // -----------------------------------------------------------------

    // A second EC test keypair, standing in for the IdP's ROTATED key: same
    // generation method as PRIV_PEM/JWKS above (openssl, P-256), never a real
    // key.
    const PRIV_PEM2: &str = "-----BEGIN PRIVATE KEY-----\nMIGHAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQgeNxFySBrzc7FM11i\nP5dhTkRMi5Wh8eqvBpoCOv0j9rShRANCAARXBmI6BE0BQJOHNYNv33PAxLpgLlVQ\nhFXwHIgvxkFjpj6I6WfrUIDiElqQ4al5pJYhGvlQIgA8eJ3R/Ah8RfG1\n-----END PRIVATE KEY-----\n";
    const ROTATED_KID: &str = "rotated-key";
    const ROTATED_JWKS: &str = r#"{"keys":[{"kty":"EC","crv":"P-256","kid":"rotated-key","x":"VwZiOgRNAUCThzWDb99zwMS6YC5VUIRV8ByIL8ZBY6Y","y":"PojpZ-tQgOISWpDhqXmkliEa-VAiADx4ndH8CHxF8bU"}]}"#;

    fn token_with_key(pem: &str, kid: &str, claims: serde_json::Value) -> String {
        let mut header = Header::new(Algorithm::ES256);
        header.kid = Some(kid.to_string());
        let key = EncodingKey::from_ec_pem(pem.as_bytes()).expect("valid key");
        encode(&header, &claims, &key).expect("encode")
    }

    fn good_claims() -> serde_json::Value {
        serde_json::json!({
            "iss": "https://idp.example",
            "aud": "genaryx-console",
            "sub": "x",
            "exp": future(),
        })
    }

    fn live_cfg(live: Arc<LiveJwks>) -> OidcConfig {
        OidcConfig {
            issuer: "https://idp.example".into(),
            audience: "genaryx-console".into(),
            keys: KeySource::Live(live),
            sub_claim: DEFAULT_SUB_CLAIM.into(),
            roles_claim: DEFAULT_ROLES_CLAIM.into(),
            admin_role: DEFAULT_ADMIN_ROLE.into(),
            approver_role: DEFAULT_APPROVER_ROLE.into(),
        }
    }

    /// A clock a test can move forward on demand, so `MAX_AGE`/`COOLDOWN`
    /// checks can be exercised without an actual wait.
    #[derive(Debug)]
    struct FakeClock(StdMutex<Instant>);

    impl FakeClock {
        fn new() -> Arc<Self> {
            Arc::new(Self(StdMutex::new(Instant::now())))
        }

        fn advance(&self, d: Duration) {
            *self.0.lock().unwrap() += d;
        }
    }

    impl Clock for FakeClock {
        fn now(&self) -> Instant {
            *self.0.lock().unwrap()
        }
    }

    /// A scripted fetcher: each call pops the next queued response (or
    /// answers a transport error once the queue runs out) and counts itself,
    /// so a test can assert exactly how many fetches happened.
    #[derive(Debug, Default)]
    struct FakeFetcher {
        calls: AtomicUsize,
        responses: StdMutex<VecDeque<Result<Vec<u8>, FetchError>>>,
    }

    impl FakeFetcher {
        fn new() -> Arc<Self> {
            Arc::new(Self::default())
        }

        fn queue(&self, resp: Result<Vec<u8>, FetchError>) {
            self.responses.lock().unwrap().push_back(resp);
        }

        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl JwksFetcher for FakeFetcher {
        async fn fetch(&self, _url: &str) -> Result<Vec<u8>, FetchError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            // A real network call always yields to the executor while in
            // flight; without this, 100 "concurrent" futures on a
            // single-threaded runtime would each run to completion in one
            // poll before the next even starts, which would show "one
            // fetch" regardless of whether single-flight is actually
            // implemented. This is what makes the concurrency test below
            // able to catch that mutant at all.
            tokio::task::yield_now().await;
            self.responses
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or_else(|| Err(FetchError::Transport("fake fetcher exhausted".into())))
        }
    }

    // -----------------------------------------------------------------
    // live source: refresh behaviour (features/oidc-live-jwks.feature)
    // -----------------------------------------------------------------

    // @test:a_rotated_key_verifies_after_the_kid_miss_refresh
    #[tokio::test]
    async fn a_rotated_key_verifies_after_the_kid_miss_refresh() {
        let fetcher = FakeFetcher::new();
        fetcher.queue(Ok(JWKS.as_bytes().to_vec()));
        let clock = FakeClock::new();
        let live = Arc::new(LiveJwks::new(
            "https://idp.example/jwks".into(),
            fetcher.clone(),
            clock.clone(),
        ));
        live.warm_up().await;
        assert_eq!(fetcher.calls(), 1);

        fetcher.queue(Ok(ROTATED_JWKS.as_bytes().to_vec()));
        clock.advance(COOLDOWN + Duration::from_secs(1));
        let cfg = live_cfg(live);
        let tok = token_with_key(PRIV_PEM2, ROTATED_KID, good_claims());
        let v = verify(&cfg, &tok)
            .await
            .expect("verifies after the rotation refresh");
        assert_eq!(v.username, "x");
        assert_eq!(
            fetcher.calls(),
            2,
            "exactly one refresh for the unknown kid"
        );
    }

    // @test:an_outage_keeps_verifying_on_the_last_good_set
    #[tokio::test]
    async fn an_outage_keeps_verifying_on_the_last_good_set() {
        let fetcher = FakeFetcher::new();
        fetcher.queue(Ok(JWKS.as_bytes().to_vec()));
        let clock = FakeClock::new();
        let live = Arc::new(LiveJwks::new(
            "https://idp.example/jwks".into(),
            fetcher.clone(),
            clock.clone(),
        ));
        live.warm_up().await;
        let good_tok = token_with(good_claims());
        let cfg = live_cfg(live);
        assert!(verify(&cfg, &good_tok).await.is_some());

        fetcher.queue(Err(FetchError::Transport("connection refused".into())));
        clock.advance(MAX_AGE + Duration::from_secs(1)); // forces the staleness refresh
        let v = verify(&cfg, &good_tok).await;
        assert!(
            v.is_some(),
            "an IdP outage must not evict the last good set"
        );
        assert_eq!(fetcher.calls(), 2, "one warm-up fetch, one failed refresh");
    }

    // @test:a_second_unknown_kid_inside_the_cooldown_makes_no_fetch
    #[tokio::test]
    async fn a_second_unknown_kid_inside_the_cooldown_makes_no_fetch() {
        let fetcher = FakeFetcher::new();
        fetcher.queue(Ok(JWKS.as_bytes().to_vec()));
        let clock = FakeClock::new();
        let live = Arc::new(LiveJwks::new(
            "https://idp.example/jwks".into(),
            fetcher.clone(),
            clock.clone(),
        ));
        live.warm_up().await;
        let cfg = live_cfg(live);

        fetcher.queue(Ok(JWKS.as_bytes().to_vec())); // still no such kid in it
        clock.advance(COOLDOWN + Duration::from_secs(1)); // past cooldown: the first miss may fetch
        let unknown1 = token_with_key(PRIV_PEM, "unknown-1", good_claims());
        assert!(verify(&cfg, &unknown1).await.is_none());
        assert_eq!(
            fetcher.calls(),
            2,
            "the first unknown kid past cooldown refreshes once"
        );

        let unknown2 = token_with_key(PRIV_PEM, "unknown-2", good_claims());
        assert!(verify(&cfg, &unknown2).await.is_none());
        assert_eq!(
            fetcher.calls(),
            2,
            "a second unknown kid inside the cooldown that just started must not fetch again"
        );
    }

    // @test:a_hundred_concurrent_unknown_kids_within_one_cooldown_make_exactly_one_fetch
    #[tokio::test]
    async fn a_hundred_concurrent_unknown_kids_within_one_cooldown_make_exactly_one_fetch() {
        let fetcher = FakeFetcher::new();
        fetcher.queue(Ok(JWKS.as_bytes().to_vec()));
        let clock = FakeClock::new();
        let live = Arc::new(LiveJwks::new(
            "https://idp.example/jwks".into(),
            fetcher.clone(),
            clock.clone(),
        ));
        live.warm_up().await;
        clock.advance(COOLDOWN + Duration::from_secs(1)); // a fetch is due

        // Whichever concurrent caller wins the single-flight fetch drains
        // this; a broken single-flight would drain more than one.
        fetcher.queue(Ok(JWKS.as_bytes().to_vec()));

        let cfg = live_cfg(live);
        let tasks = (0..100u32).map(|i| {
            let cfg = &cfg;
            async move {
                let tok = token_with_key(PRIV_PEM, &format!("unknown-{i}"), good_claims());
                verify(cfg, &tok).await
            }
        });
        let results = futures_util::future::join_all(tasks).await;
        assert!(results.iter().all(Option::is_none));
        assert_eq!(
            fetcher.calls(),
            2,
            "one warm-up fetch, one shared refresh for all 100 concurrent misses"
        );
    }

    // @test:after_max_age_a_refresh_that_drops_a_key_fails_that_keys_token
    #[tokio::test]
    async fn after_max_age_a_refresh_that_drops_a_key_fails_that_keys_token() {
        let fetcher = FakeFetcher::new();
        fetcher.queue(Ok(JWKS.as_bytes().to_vec()));
        let clock = FakeClock::new();
        let live = Arc::new(LiveJwks::new(
            "https://idp.example/jwks".into(),
            fetcher.clone(),
            clock.clone(),
        ));
        live.warm_up().await;
        let cfg = live_cfg(live);
        let old_tok = token_with(good_claims());
        assert!(verify(&cfg, &old_tok).await.is_some());

        fetcher.queue(Ok(ROTATED_JWKS.as_bytes().to_vec())); // the IdP dropped the old key
        clock.advance(MAX_AGE + Duration::from_secs(1));

        assert!(
            verify(&cfg, &old_tok).await.is_none(),
            "a key the IdP removed must stop verifying once a refresh SUCCEEDS"
        );
        let new_tok = token_with_key(PRIV_PEM2, ROTATED_KID, good_claims());
        assert!(
            verify(&cfg, &new_tok).await.is_some(),
            "the rotated key must be the one now trusted"
        );
        assert_eq!(fetcher.calls(), 2);
    }

    // @test:hostile_jwks_bodies_are_refused_and_the_last_good_set_stays
    #[tokio::test]
    async fn hostile_jwks_bodies_are_refused_and_the_last_good_set_stays() {
        let hostile_bodies: [(&str, Vec<u8>); 4] = [
            ("invalid json", b"not json at all".to_vec()),
            ("empty keys", br#"{"keys":[]}"#.to_vec()),
            (
                "an oct key",
                br#"{"keys":[{"kty":"oct","kid":"x","k":"c2VjcmV0"}]}"#.to_vec(),
            ),
            (
                "a key carrying d",
                br#"{"keys":[{"kty":"RSA","kid":"leaky","n":"abc","e":"AQAB","d":"leaked"}]}"#
                    .to_vec(),
            ),
        ];

        for (label, body) in hostile_bodies {
            let fetcher = FakeFetcher::new();
            fetcher.queue(Ok(JWKS.as_bytes().to_vec()));
            let clock = FakeClock::new();
            let live = Arc::new(LiveJwks::new(
                "https://idp.example/jwks".into(),
                fetcher.clone(),
                clock.clone(),
            ));
            live.warm_up().await;
            let cfg = live_cfg(live);
            let good_tok = token_with(good_claims());
            assert!(verify(&cfg, &good_tok).await.is_some(), "{label}: setup");

            fetcher.queue(Ok(body));
            // Past cooldown but well under MAX_AGE, so the ONLY reason a
            // fetch happens here is the unknown kid below - keeps this case
            // from also tripping the (unconditional) staleness refresh a
            // second time on the follow-up good_tok check.
            clock.advance(COOLDOWN + Duration::from_secs(1));
            let unknown = token_with_key(PRIV_PEM, "does-not-exist", good_claims());
            assert!(
                verify(&cfg, &unknown).await.is_none(),
                "{label}: unknown kid never verifies"
            );
            assert_eq!(
                fetcher.calls(),
                2,
                "{label}: exactly one refresh attempt for the unknown kid"
            );
            assert!(
                verify(&cfg, &good_tok).await.is_some(),
                "{label}: a hostile body must not evict the last good set"
            );
            assert_eq!(
                fetcher.calls(),
                2,
                "{label}: the good lookup needed no further fetch"
            );
        }
    }

    // -----------------------------------------------------------------
    // validate_jwks_bytes: pure, direct
    // -----------------------------------------------------------------

    #[test]
    fn validate_rejects_invalid_json() {
        assert_eq!(
            validate_jwks_bytes(b"not json").unwrap_err(),
            RefusedSet::InvalidJson
        );
    }

    #[test]
    fn validate_rejects_a_non_object_top_level() {
        assert_eq!(
            validate_jwks_bytes(b"[1,2,3]").unwrap_err(),
            RefusedSet::NotAnObject
        );
    }

    #[test]
    fn validate_rejects_missing_or_non_array_keys() {
        assert_eq!(
            validate_jwks_bytes(br#"{"keys":"nope"}"#).unwrap_err(),
            RefusedSet::KeysMissingOrNotArray
        );
        assert_eq!(
            validate_jwks_bytes(br#"{}"#).unwrap_err(),
            RefusedSet::KeysMissingOrNotArray
        );
    }

    #[test]
    fn validate_rejects_empty_keys() {
        assert_eq!(
            validate_jwks_bytes(br#"{"keys":[]}"#).unwrap_err(),
            RefusedSet::EmptyKeys
        );
    }

    #[test]
    fn validate_rejects_a_key_entry_that_is_not_an_object() {
        assert_eq!(
            validate_jwks_bytes(br#"{"keys":["not-an-object"]}"#).unwrap_err(),
            RefusedSet::KeyNotAnObject
        );
    }

    #[test]
    fn validate_rejects_an_oct_key() {
        let err = validate_jwks_bytes(br#"{"keys":[{"kty":"oct","kid":"x","k":"c2VjcmV0"}]}"#)
            .unwrap_err();
        assert!(matches!(err, RefusedSet::UnsupportedKeyType(ref t) if t == "oct"));
    }

    #[test]
    fn validate_rejects_an_okp_key() {
        let err =
            validate_jwks_bytes(br#"{"keys":[{"kty":"OKP","crv":"Ed25519","kid":"x","x":"abc"}]}"#)
                .unwrap_err();
        assert!(matches!(err, RefusedSet::UnsupportedKeyType(ref t) if t == "OKP"));
    }

    #[test]
    fn validate_rejects_a_leaked_private_rsa_exponent() {
        // `n`/`e` alone would be an ordinary, acceptable public RSA key; the
        // `d` is what must refuse the whole set even though the key TYPE is
        // allowed - jsonwebtoken's own RSAKeyParameters would silently drop
        // `d` on a typed parse, which is exactly why this check runs first.
        let body = br#"{"keys":[{"kty":"RSA","kid":"leaky","n":"abc","e":"AQAB","d":"nope"}]}"#;
        let err = validate_jwks_bytes(body).unwrap_err();
        assert!(matches!(
            err,
            RefusedSet::PrivateKeyMember { member: "d", .. }
        ));
    }

    #[test]
    fn validate_rejects_every_forbidden_member_individually() {
        for member in FORBIDDEN_PRIVATE_MEMBERS {
            let body = format!(
                r#"{{"keys":[{{"kty":"EC","crv":"P-256","kid":"x","x":"a","y":"b","{member}":"z"}}]}}"#
            );
            let err = validate_jwks_bytes(body.as_bytes()).unwrap_err();
            assert!(
                matches!(err, RefusedSet::PrivateKeyMember { member: m, .. } if m == *member),
                "member {member}: got {err:?}"
            );
        }
    }

    #[test]
    fn validate_accepts_a_clean_ec_set() {
        assert!(validate_jwks_bytes(JWKS.as_bytes()).is_ok());
    }

    /// A cheap, seeded pseudo-random generator (splitmix64) so the sweep below
    /// is reproducible without pulling a `rand` dependency into this
    /// function - the workspace's `rand` crate is already used elsewhere in
    /// this file's tests, but a hand-rolled generator keeps this one
    /// self-contained and exact-seed-reproducible across `rand` versions.
    struct SplitMix64(u64);

    impl SplitMix64 {
        fn next(&mut self) -> u64 {
            self.0 = self.0.wrapping_add(0x9E3779B97F4A7C15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
            z ^ (z >> 31)
        }

        fn below(&mut self, n: usize) -> usize {
            (self.next() as usize) % n.max(1)
        }
    }

    /// One of several ways a JWKS body can be hostile or merely garbage,
    /// selected and parameterised by `seed` (module doc / CLAUDE.md invariant
    /// 11: "sweep the range with seeds instead of picking three values").
    fn hostile_body_for_seed(seed: u64) -> Vec<u8> {
        let mut r = SplitMix64(seed.wrapping_mul(0x2545F4914F6CDD1D).wrapping_add(1));
        match r.below(8) {
            0 => {
                // Raw random bytes, not even UTF-8-safe JSON.
                (0..r.below(200)).map(|_| (r.next() % 256) as u8).collect()
            }
            1 => b"{".to_vec(), // truncated
            2 => br#"{"keys": null}"#.to_vec(),
            3 => br#"{"keys": [null, 42, true, "x", []]}"#.to_vec(),
            4 => {
                // A deeply (but boundedly) nested array in place of "keys".
                let depth = 1 + r.below(500);
                let mut s = String::from(r#"{"keys":"#);
                s.push_str(&"[".repeat(depth));
                s.push_str(&"]".repeat(depth));
                s.push('}');
                s.into_bytes()
            }
            5 => {
                // A key object with a random assortment of forbidden members
                // and a random (possibly allowed, possibly not) kty.
                let ktys = ["RSA", "EC", "oct", "OKP", "", "rsa", "EC "];
                let kty = ktys[r.below(ktys.len())];
                let mut obj = format!(r#"{{"kty":"{kty}","kid":"seed-{seed}""#);
                for member in FORBIDDEN_PRIVATE_MEMBERS {
                    if r.below(2) == 0 {
                        obj.push_str(&format!(r#","{member}":"x""#));
                    }
                }
                obj.push('}');
                format!(r#"{{"keys":[{obj}]}}"#).into_bytes()
            }
            6 => {
                // Unicode / escape-heavy strings in a shape that otherwise
                // parses.
                let body = serde_json::json!({
                    "keys": [{"kty": "EC", "crv": "P-256", "kid": "\u{1F600}\"\\\n\t", "x": "a", "y": "b"}]
                });
                serde_json::to_vec(&body).unwrap()
            }
            _ => {
                // A huge numeric literal, which some JSON parsers mishandle.
                format!(
                    r#"{{"keys":[{{"kty":"RSA","kid":"x","n":"a","e":{}}}]}}"#,
                    "1".repeat(2000)
                )
                .into_bytes()
            }
        }
    }

    #[test]
    fn two_hundred_seeds_of_hostile_jwks_bodies_never_panic() {
        for seed in 0u64..200 {
            let body = hostile_body_for_seed(seed);
            let result = std::panic::catch_unwind(|| validate_jwks_bytes(&body));
            assert!(
                result.is_ok(),
                "seed {seed} panicked on body {:?}",
                String::from_utf8_lossy(&body)
            );
        }
    }

    // -----------------------------------------------------------------
    // resolve_jwks_source: the two "refuse to start" cases
    // -----------------------------------------------------------------

    // @test:both_jwks_sources_set_refuses_to_start
    #[test]
    fn both_jwks_sources_set_refuses_to_start() {
        let err = resolve_jwks_source(
            Some(JWKS.to_string()),
            Some("https://idp.example/jwks".to_string()),
        )
        .unwrap_err();
        assert_eq!(err, StartupRefusal::BothJwksSourcesSet);
    }

    // @test:a_plain_http_jwks_url_refuses_to_start
    #[test]
    fn a_plain_http_jwks_url_refuses_to_start() {
        let err =
            resolve_jwks_source(None, Some("http://idp.example/jwks".to_string())).unwrap_err();
        assert!(matches!(
            err,
            StartupRefusal::JwksUrlNotHttps(ref u) if u == "http://idp.example/jwks"
        ));
    }

    #[test]
    fn no_jwks_source_leaves_oidc_unconfigured() {
        assert!(matches!(
            resolve_jwks_source(None, None),
            Ok(KeySourceChoice::Unconfigured)
        ));
    }

    #[test]
    fn an_https_url_alone_is_accepted() {
        assert!(matches!(
            resolve_jwks_source(None, Some("https://idp.example/jwks".to_string())),
            Ok(KeySourceChoice::LiveUrl(_))
        ));
    }

    #[test]
    fn a_static_source_alone_is_accepted() {
        assert!(matches!(
            resolve_jwks_source(Some(JWKS.to_string()), None),
            Ok(KeySourceChoice::Static(_))
        ));
    }

    // -----------------------------------------------------------------
    // the REAL fetcher's own seam: a local plain-http server, reachable only
    // from these tests (never from configuration - `resolve_jwks_source`
    // above is the only https gate, and it runs at a different layer).
    // -----------------------------------------------------------------

    /// A minimal HTTP/1.1 server that answers each of `responses`, one per
    /// accepted connection, in order, then stops. Enough for `ReqwestFetcher`
    /// (a bare GET, no auth, no keep-alive assumed) without a real IdP.
    async fn serve_sequence(responses: Vec<Vec<u8>>) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind a local test server");
        let addr = listener.local_addr().expect("local addr");
        tokio::spawn(async move {
            for resp in responses {
                let Ok((mut sock, _)) = listener.accept().await else {
                    break;
                };
                let mut buf = [0u8; 4096];
                let _ = tokio::io::AsyncReadExt::read(&mut sock, &mut buf).await;
                let _ = tokio::io::AsyncWriteExt::write_all(&mut sock, &resp).await;
                let _ = tokio::io::AsyncWriteExt::shutdown(&mut sock).await;
            }
        });
        format!("http://{addr}")
    }

    async fn serve_once(response: Vec<u8>) -> String {
        serve_sequence(vec![response]).await
    }

    fn http_ok_body(body: &str) -> Vec<u8> {
        format!(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
            body.len(),
            body
        )
        .into_bytes()
    }

    // @test:the_real_fetcher_refuses_a_redirect
    #[tokio::test]
    async fn the_real_fetcher_refuses_a_redirect() {
        let url = serve_once(
            b"HTTP/1.1 302 Found\r\nlocation: http://example.invalid/\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
                .to_vec(),
        )
        .await;
        let err = ReqwestFetcher.fetch(&url).await.unwrap_err();
        assert_eq!(err, FetchError::Redirected);
    }

    // @test:the_real_fetcher_refuses_an_oversized_body
    #[tokio::test]
    async fn the_real_fetcher_refuses_an_oversized_body() {
        let mut resp = b"HTTP/1.1 200 OK\r\nconnection: close\r\n\r\n".to_vec();
        resp.extend(std::iter::repeat_n(b'a', MAX_JWKS_BODY_BYTES + 1024));
        let url = serve_once(resp).await;
        let err = ReqwestFetcher.fetch(&url).await.unwrap_err();
        assert_eq!(err, FetchError::TooLarge);
    }

    // @test:a_body_over_the_cap_through_the_real_fetcher_keeps_the_last_good_set
    #[tokio::test]
    async fn a_body_over_the_cap_through_the_real_fetcher_keeps_the_last_good_set() {
        let good = http_ok_body(JWKS);
        let mut oversized = b"HTTP/1.1 200 OK\r\nconnection: close\r\n\r\n".to_vec();
        oversized.extend(std::iter::repeat_n(b'a', MAX_JWKS_BODY_BYTES + 1024));
        let url = serve_sequence(vec![good, oversized]).await;

        let clock = FakeClock::new();
        let live = Arc::new(LiveJwks::new(url, Arc::new(ReqwestFetcher), clock.clone()));
        live.warm_up().await;
        let cfg = live_cfg(live);
        let good_tok = token_with(good_claims());
        assert!(
            verify(&cfg, &good_tok).await.is_some(),
            "the warmed-up key must verify"
        );

        clock.advance(MAX_AGE + Duration::from_secs(1));
        let v = verify(&cfg, &good_tok).await;
        assert!(
            v.is_some(),
            "an oversized refresh body, from the REAL fetcher, must not evict the last good set"
        );
    }
}
