//! Read proxy + mutation pass-through (docs/PHASE5.md "proxy" module;
//! itrat-console/13 D12.2b/c).
//!
//! Two very different trust shapes share this file because they share a
//! wire contract (the same `/v1` path space, D12.3: "The relay presents the
//! same `/v1` path space to the phone precisely for this"):
//!
//! - **Reads** ([`summary_handler`]): the phone's OWN bearer travels straight
//!   through to the Cloud, which resolves it to (org, plan) itself
//!   (`http.rs::org_for`). The relay adds no ambient authority here (D12.3:
//!   "the relay adds no ambient authority to reads either") and therefore
//!   checks nothing beyond forwarding -- a bad/missing bearer simply comes
//!   back as the Cloud's own 401. Deliberately an explicit allowlist of ONE
//!   path (`/v1/summary`), not a wildcard `/v1/*` forward: a wildcard would
//!   quietly reopen the exact `/v1/runs` 9k-row choke the whole relay design
//!   exists to close (docs/PHASE5.md, D12.2b step 5).
//! - **Mutations** ([`mutation_passthrough`]): forwarded VERBATIM (same
//!   method, path, body, `X-Fuse-*` headers) so the phone's ES256 signature
//!   transfers with no re-canonicalization (D12.2c step 3) -- the relay
//!   deliberately does NOT re-derive or re-verify the canonical string
//!   itself (that would reintroduce exactly the re-canonicalization risk the
//!   architecture avoids by design; the Cloud remains the sole verifier).
//!   Ahead of forwarding it checks the three things D12.2c step 3 actually
//!   asks for: "device row exists, token matches (constant-time), rate limit
//!   OK" -- via the registry and [`crate::ratelimit::RateLimiter`].

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};

#[derive(Debug, thiserror::Error)]
pub enum ProxyError {
    #[error("unauthorized")]
    Unauthorized,
    /// The device is genuinely paired, but this mutation is not one its
    /// surface may perform. See [`kind_may_mutate`].
    #[error("this device may not perform that action")]
    Forbidden,
    #[error("rate limit exceeded")]
    RateLimited,
    #[error("internal: {0}")]
    Internal(String),
}

impl IntoResponse for ProxyError {
    fn into_response(self) -> Response {
        let (status, code) = match &self {
            ProxyError::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized"),
            ProxyError::Forbidden => (StatusCode::FORBIDDEN, "forbidden_for_device"),
            ProxyError::RateLimited => (StatusCode::TOO_MANY_REQUESTS, "rate_limited"),
            ProxyError::Internal(_) => (StatusCode::INTERNAL_SERVER_ERROR, "internal_error"),
        };
        (status, Json(serde_json::json!({ "error": code }))).into_response()
    }
}

/// May a device of this kind perform the mutation at `path`?
///
/// ## Why the wrist is narrower than the pocket
///
/// Both devices are admitted by the same operator and both sign with their own
/// key, so this is not about trusting one less. It is about what each surface
/// is FOR, and about what a 40mm screen under time pressure is a good place to
/// decide.
///
/// - **kill** is the whole reason the watch exists: something is burning, stop
///   it now. Allowed on both.
/// - **incident ack** is a pager-native action, "I have seen this, stop paging
///   me". Allowed on both.
/// - **budget** is a decision to SPEND MORE MONEY. It wants context, a bigger
///   screen and a moment's thought, none of which a wrist has. Phone only.
///
/// ## What this is and is not
///
/// This is enforced HERE, at the relay, not at the Cloud. The Cloud's own role
/// model is only `admin` or `viewer` (`tokenfuse/crates/cloud/src/keys.rs`),
/// and neither expresses "may kill but may not re-budget", so the Cloud cannot
/// carry this distinction today. The relay is the only door to the Cloud (which
/// is bound to loopback), so this is a real boundary in the deployed shape, but
/// it is a relay-enforced one. Say exactly that in any writeup: do not claim
/// the watch's credential is intrinsically weaker at the Cloud, because it is
/// not. Making it intrinsically weaker needs a Cloud-side capability grant,
/// which is a separate change.
pub fn kind_may_mutate(kind: crate::registry::DeviceKind, path: &str) -> bool {
    use crate::registry::DeviceKind;
    // Match on the concrete path shape rather than a prefix, so a future route
    // added to the router is denied by default until it is listed here.
    let is_kill = path.starts_with("/v1/runs/") && path.ends_with("/kill");
    let is_budget = path.starts_with("/v1/runs/") && path.ends_with("/budget");
    let is_ack = path.starts_with("/v1/incidents/") && path.ends_with("/ack");
    match kind {
        DeviceKind::Phone => is_kill || is_budget || is_ack,
        DeviceKind::Watch => is_kill || is_ack,
    }
}

/// The bearer token from `Authorization`, with or without the `Bearer `
/// prefix -- mirrors `tokenfuse-cloud::http.rs::bearer` exactly, since the
/// relay speaks the same convention on its own public routes.
pub fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    let raw = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let token = raw.strip_prefix("Bearer ").unwrap_or(raw).trim();
    (!token.is_empty()).then_some(token)
}

/// `GET /v1/summary`, proxied read (see module docs: the only allowlisted
/// read path in W1). The phone's own `Authorization` header travels through
/// unchanged; every other header is dropped rather than forwarded blind.
pub async fn summary_handler(
    State(state): State<crate::PublicState>,
    headers: HeaderMap,
) -> Response {
    let mut req = state
        .http
        .get(format!("{}/v1/summary", state.cloud_base_url));
    if let Some(auth) = headers.get(header::AUTHORIZATION) {
        req = req.header(header::AUTHORIZATION, auth.clone());
    }
    match req.send().await {
        Ok(resp) => reqwest_response_to_axum(resp).await,
        Err(e) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({ "error": "upstream_unavailable", "detail": e.to_string() })),
        )
            .into_response(),
    }
}

/// One handler registered at all three mutation routes
/// (`/v1/runs/{run}/kill`, `/v1/runs/{run}/budget`, `/v1/incidents/{id}/ack`):
/// `uri.path()` is already the literal, concrete incoming request path (no
/// path param is ever re-assembled from a decoded `Path<String>`), so a
/// single code path forwards every one of them byte-identically.
pub async fn mutation_passthrough(
    State(state): State<crate::PublicState>,
    headers: HeaderMap,
    uri: Uri,
    body: Bytes,
) -> Result<Response, ProxyError> {
    let token = bearer_token(&headers).ok_or(ProxyError::Unauthorized)?;
    let device = state
        .registry
        .verify_bearer(token)
        .map_err(|e| ProxyError::Internal(e.to_string()))?
        .ok_or(ProxyError::Unauthorized)?;

    // What this surface is allowed to do, before spending any rate-limit
    // budget or touching the Cloud. A denial here is a 403 and is deliberately
    // distinguishable from a 401: the device IS paired, it just may not do
    // this, and telling it so plainly is what lets the app hide the control
    // rather than offer a button that always fails.
    if !kind_may_mutate(device.kind, uri.path()) {
        eprintln!(
            "genaryx-relay: proxy: refused {} on {} (paired {} may not perform it)",
            device.device_id,
            uri.path(),
            device.kind.as_str()
        );
        return Err(ProxyError::Forbidden);
    }

    if !state.mutation_rate_limiter.check(&device.device_id) {
        return Err(ProxyError::RateLimited);
    }
    if let Err(e) = state
        .registry
        .touch_last_seen(&device.device_id, crate::exceptions::now_unix())
    {
        eprintln!("genaryx-relay: proxy: touch_last_seen failed (non-fatal): {e}");
    }

    // Every header the Cloud's signature verification and bearer auth
    // actually consult (`http.rs::verify_device_signature`/`bearer`), plus
    // `content-type` for a correct body parse on the budget mutation.
    // Nothing else is forwarded -- and nothing about the body or these
    // values is ever modified.
    const FORWARD_HEADERS: &[&str] = &[
        "authorization",
        "x-fuse-device",
        "x-fuse-ts",
        "x-fuse-nonce",
        "x-fuse-sig",
        "content-type",
    ];
    let mut req = state
        .http
        .post(format!("{}{}", state.cloud_base_url, uri.path()));
    for name in FORWARD_HEADERS {
        if let Some(v) = headers.get(*name) {
            req = req.header(*name, v.clone());
        }
    }
    let resp = req
        .body(body)
        .send()
        .await
        .map_err(|e| ProxyError::Internal(format!("forwarding to Cloud: {e}")))?;
    Ok(reqwest_response_to_axum(resp).await)
}

/// Turn a `reqwest::Response` into an `axum::response::Response` with the
/// same status, body, and content-type -- used by both the read proxy and
/// the mutation pass-through so a caller (phone) sees exactly what the
/// Cloud said, verbatim.
async fn reqwest_response_to_axum(resp: reqwest::Response) -> Response {
    let status = StatusCode::from_u16(resp.status().as_u16()).unwrap_or(StatusCode::BAD_GATEWAY);
    let content_type = resp.headers().get(header::CONTENT_TYPE).cloned();
    let bytes = match resp.bytes().await {
        Ok(b) => b,
        Err(e) => {
            return (
                StatusCode::BAD_GATEWAY,
                Json(
                    serde_json::json!({ "error": "upstream_read_failed", "detail": e.to_string() }),
                ),
            )
                .into_response();
        }
    };
    let mut builder = Response::builder().status(status);
    if let Some(ct) = content_type {
        builder = builder.header(header::CONTENT_TYPE, ct);
    }
    match builder.body(axum::body::Body::from(bytes)) {
        Ok(response) => response,
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers_with_auth(value: &str) -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert(header::AUTHORIZATION, value.parse().unwrap());
        h
    }

    #[test]
    fn bearer_token_strips_the_bearer_prefix() {
        assert_eq!(
            bearer_token(&headers_with_auth("Bearer abc123")),
            Some("abc123")
        );
    }

    #[test]
    fn bearer_token_accepts_a_raw_token_with_no_prefix() {
        assert_eq!(bearer_token(&headers_with_auth("abc123")), Some("abc123"));
    }

    #[test]
    fn bearer_token_is_none_for_empty_or_missing_header() {
        assert_eq!(bearer_token(&HeaderMap::new()), None);
        assert_eq!(bearer_token(&headers_with_auth("")), None);
        assert_eq!(bearer_token(&headers_with_auth("Bearer ")), None);
    }

    // ---- mutation_passthrough: verbatim forwarding, end to end -------------
    //
    // These stand up a tiny local "fake Cloud" (a real axum server on
    // 127.0.0.1) that records exactly what it received, so the assertions
    // below are against genuine forwarded bytes/headers over a real HTTP
    // round trip, not a mocked call.

    #[derive(Debug, Default, Clone)]
    struct Captured {
        method: String,
        path: String,
        body: Vec<u8>,
        headers: HeaderMap,
    }

    /// Start a fake Cloud that records the one request it receives and
    /// answers `200 {"ok":true}`. Returns its base URL and the capture slot.
    async fn spawn_fake_cloud() -> (String, std::sync::Arc<std::sync::Mutex<Option<Captured>>>) {
        let captured: std::sync::Arc<std::sync::Mutex<Option<Captured>>> =
            std::sync::Arc::new(std::sync::Mutex::new(None));
        let captured_for_handler = captured.clone();

        let app = axum::Router::new().fallback(move |req: axum::extract::Request| {
            let captured = captured_for_handler.clone();
            async move {
                let method = req.method().to_string();
                let path = req.uri().path().to_string();
                let headers = req.headers().clone();
                let body = axum::body::to_bytes(req.into_body(), usize::MAX)
                    .await
                    .unwrap_or_default();
                *captured.lock().unwrap() = Some(Captured {
                    method,
                    path,
                    body: body.to_vec(),
                    headers,
                });
                (StatusCode::OK, Json(serde_json::json!({ "ok": true })))
            }
        });

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        (format!("http://{addr}"), captured)
    }

    fn state_with_paired_device(cloud_base_url: String) -> crate::PublicState {
        let registry = std::sync::Arc::new(crate::registry::Registry::open_in_memory().unwrap());
        registry
            .insert_paired_device(
                crate::registry::DeviceKind::Phone,
                crate::registry::NewDevice {
                    device_id: "dev-1".to_string(),
                    name: "iPhone".to_string(),
                    platform: "ios".to_string(),
                    org: "acme".to_string(),
                    role: "admin".to_string(),
                    device_token: "tok-1".to_string(),
                    paired_at_unix: 1000,
                },
            )
            .unwrap();
        crate::PublicState {
            registry,
            engine: std::sync::Arc::new(crate::exceptions::ExceptionEngine::new("acme", 0.8, 600)),
            http: reqwest::Client::new(),
            cloud_base_url,
            public_advertise_url: "https://127.0.0.1:8443".to_string(),
            mutation_rate_limiter: std::sync::Arc::new(crate::ratelimit::RateLimiter::new(
                100,
                std::time::Duration::from_secs(60),
            )),
            pairing_rate_limiter: std::sync::Arc::new(crate::ratelimit::RateLimiter::new(
                100,
                std::time::Duration::from_secs(60),
            )),
        }
    }

    #[tokio::test]
    async fn mutation_passthrough_forwards_method_path_body_and_fuse_headers_verbatim() {
        let (cloud_base_url, captured) = spawn_fake_cloud().await;
        let state = state_with_paired_device(cloud_base_url);

        let mut headers = HeaderMap::new();
        headers.insert(header::AUTHORIZATION, "Bearer tok-1".parse().unwrap());
        headers.insert("x-fuse-device", "dev-1".parse().unwrap());
        headers.insert("x-fuse-ts", "1700000000".parse().unwrap());
        headers.insert("x-fuse-nonce", "abc123".parse().unwrap());
        headers.insert("x-fuse-sig", "deadbeef==".parse().unwrap());
        headers.insert(header::CONTENT_TYPE, "application/json".parse().unwrap());
        // Not in the forward allowlist: must NOT reach the Cloud.
        headers.insert("x-should-not-forward", "nope".parse().unwrap());

        let uri: Uri = "/v1/runs/spike2-e2e/budget".parse().unwrap();
        let body = Bytes::from_static(br#"{"budget_usd":12.5}"#);

        let response = mutation_passthrough(State(state), headers, uri, body)
            .await
            .expect("forwards successfully");
        assert_eq!(response.status(), StatusCode::OK);

        let cap = captured
            .lock()
            .unwrap()
            .take()
            .expect("the fake Cloud received exactly one request");
        assert_eq!(cap.method, "POST");
        assert_eq!(
            cap.path, "/v1/runs/spike2-e2e/budget",
            "path forwarded verbatim"
        );
        assert_eq!(
            cap.body, br#"{"budget_usd":12.5}"#,
            "body forwarded verbatim, unmodified"
        );
        assert_eq!(
            cap.headers.get(header::AUTHORIZATION).unwrap(),
            "Bearer tok-1"
        );
        assert_eq!(cap.headers.get("x-fuse-device").unwrap(), "dev-1");
        assert_eq!(cap.headers.get("x-fuse-ts").unwrap(), "1700000000");
        assert_eq!(cap.headers.get("x-fuse-nonce").unwrap(), "abc123");
        assert_eq!(cap.headers.get("x-fuse-sig").unwrap(), "deadbeef==");
        assert!(
            cap.headers.get("x-should-not-forward").is_none(),
            "only the allowlisted headers are forwarded, never the full set"
        );
    }

    /// The hop the phone's signature depends on and that no review could
    /// observe until now: a percent-encoded id must reach the Cloud with the
    /// SAME bytes it left the phone with.
    ///
    /// The phone signs the encoded path (`String.asPathSegment`, mobile #15)
    /// and the Cloud verifies over `uri.path()`, the raw encoded path
    /// (`http.rs::kill`). The relay sits between them, so if it decoded,
    /// re-encoded, or normalized that path by even one byte, every mutation
    /// on an id with a reserved character would come back
    /// `403 signature_invalid` - and the affected mutation is `kill`.
    /// `Uri::path()` hands back the raw path and `url` never re-encodes an
    /// existing `%`, so the forward is verbatim; this asserts it against a
    /// real HTTP round trip instead of leaving it to the next live run.
    #[tokio::test]
    async fn mutation_passthrough_forwards_a_percent_encoded_id_byte_for_byte() {
        let (cloud_base_url, captured) = spawn_fake_cloud().await;
        let state = state_with_paired_device(cloud_base_url);

        let mut headers = HeaderMap::new();
        headers.insert(header::AUTHORIZATION, "Bearer tok-1".parse().unwrap());
        headers.insert("x-fuse-device", "dev-1".parse().unwrap());

        // `зупинка #7 a/b`, encoded exactly as the phone and the desktop
        // console both encode one path segment.
        let signed_path = "/v1/runs/%D0%B7%D1%83%D0%BF%D0%B8%D0%BD%D0%BA%D0%B0%20%237%20a%2Fb/kill";
        let uri: Uri = signed_path.parse().unwrap();

        let response = mutation_passthrough(State(state), headers, uri, Bytes::new())
            .await
            .expect("forwards successfully");
        assert_eq!(response.status(), StatusCode::OK);

        let cap = captured
            .lock()
            .unwrap()
            .take()
            .expect("the fake Cloud received exactly one request");
        assert_eq!(
            cap.path, signed_path,
            "the encoded path must arrive unchanged, or the phone's signature no longer verifies"
        );
    }

    #[tokio::test]
    async fn mutation_passthrough_rejects_wrong_bearer_before_ever_reaching_the_cloud() {
        let (cloud_base_url, captured) = spawn_fake_cloud().await;
        let state = state_with_paired_device(cloud_base_url);

        let mut headers = HeaderMap::new();
        headers.insert(header::AUTHORIZATION, "Bearer wrong-token".parse().unwrap());
        headers.insert("x-fuse-device", "dev-1".parse().unwrap());
        let uri: Uri = "/v1/runs/r1/kill".parse().unwrap();

        let err = mutation_passthrough(State(state), headers, uri, Bytes::new())
            .await
            .expect_err("a non-matching bearer must be rejected");
        assert!(matches!(err, ProxyError::Unauthorized));
        assert!(
            captured.lock().unwrap().is_none(),
            "the relay must reject BEFORE forwarding anything to the Cloud"
        );
    }

    /// The same state, plus a paired WATCH whose token is `tok-w`.
    fn state_with_phone_and_watch(cloud_base_url: String) -> crate::PublicState {
        let state = state_with_paired_device(cloud_base_url);
        state
            .registry
            .insert_paired_device(
                crate::registry::DeviceKind::Watch,
                crate::registry::NewDevice {
                    device_id: "dev-w".to_string(),
                    name: "Apple Watch".to_string(),
                    platform: "watchos".to_string(),
                    org: "acme".to_string(),
                    role: "admin".to_string(),
                    device_token: "tok-w".to_string(),
                    paired_at_unix: 1000,
                },
            )
            .unwrap();
        state
    }

    fn signed_headers(token: &str, device: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            format!("Bearer {token}").parse().unwrap(),
        );
        headers.insert("x-fuse-device", device.parse().unwrap());
        headers
    }

    #[test]
    fn the_wrist_may_stop_things_but_may_not_authorize_more_spend() {
        use crate::registry::DeviceKind::{Phone, Watch};
        // Kill and ack: both surfaces. Budget: the pocket only.
        assert!(kind_may_mutate(Watch, "/v1/runs/r1/kill"));
        assert!(kind_may_mutate(Watch, "/v1/incidents/i1/ack"));
        assert!(!kind_may_mutate(Watch, "/v1/runs/r1/budget"));

        assert!(kind_may_mutate(Phone, "/v1/runs/r1/kill"));
        assert!(kind_may_mutate(Phone, "/v1/incidents/i1/ack"));
        assert!(kind_may_mutate(Phone, "/v1/runs/r1/budget"));

        // Anything not explicitly listed is denied for BOTH, so a route added
        // to the router later cannot quietly inherit authority.
        for path in [
            "/v1/runs/r1/resume",
            "/v1/policies/p1/approve",
            "/v1/summary",
            "/",
        ] {
            assert!(!kind_may_mutate(Phone, path), "phone: {path}");
            assert!(!kind_may_mutate(Watch, path), "watch: {path}");
        }
    }

    #[tokio::test]
    async fn a_watch_budget_change_is_refused_before_ever_reaching_the_cloud() {
        let (cloud_base_url, captured) = spawn_fake_cloud().await;
        let state = state_with_phone_and_watch(cloud_base_url);

        let err = mutation_passthrough(
            State(state),
            signed_headers("tok-w", "dev-w"),
            "/v1/runs/r1/budget".parse().unwrap(),
            Bytes::from_static(b"{\"budget_usd\":999.0}"),
        )
        .await
        .expect_err("the watch must not be able to raise a budget");
        assert!(matches!(err, ProxyError::Forbidden));
        assert!(
            captured.lock().unwrap().is_none(),
            "the refusal must happen at the relay, not by asking the Cloud"
        );
    }

    #[tokio::test]
    async fn a_watch_kill_is_forwarded_like_any_other() {
        let (cloud_base_url, captured) = spawn_fake_cloud().await;
        let state = state_with_phone_and_watch(cloud_base_url);

        let resp = mutation_passthrough(
            State(state),
            signed_headers("tok-w", "dev-w"),
            "/v1/runs/reconciliation-batch-eod-002-LIVE/kill"
                .parse()
                .unwrap(),
            Bytes::new(),
        )
        .await
        .expect("a kill from the wrist is the whole point");
        assert_eq!(resp.status(), StatusCode::OK);

        let got = captured.lock().unwrap().clone().expect("Cloud was called");
        assert_eq!(got.path, "/v1/runs/reconciliation-batch-eod-002-LIVE/kill");
        assert_eq!(
            got.headers.get("x-fuse-device").unwrap(),
            "dev-w",
            "the watch's own signature identity must survive the hop"
        );
    }

    #[tokio::test]
    async fn the_phone_may_still_change_a_budget() {
        let (cloud_base_url, captured) = spawn_fake_cloud().await;
        let state = state_with_phone_and_watch(cloud_base_url);

        let resp = mutation_passthrough(
            State(state),
            signed_headers("tok-1", "dev-1"),
            "/v1/runs/r1/budget".parse().unwrap(),
            Bytes::from_static(b"{\"budget_usd\":10.0}"),
        )
        .await
        .expect("the phone keeps the full mutation set");
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(captured.lock().unwrap().is_some());
    }

    #[tokio::test]
    async fn mutation_passthrough_enforces_the_rate_limit_before_forwarding() {
        let (cloud_base_url, _captured) = spawn_fake_cloud().await;
        let mut state = state_with_paired_device(cloud_base_url);
        state.mutation_rate_limiter = std::sync::Arc::new(crate::ratelimit::RateLimiter::new(
            1,
            std::time::Duration::from_secs(60),
        ));

        let headers = || {
            let mut h = HeaderMap::new();
            h.insert(header::AUTHORIZATION, "Bearer tok-1".parse().unwrap());
            h.insert("x-fuse-device", "dev-1".parse().unwrap());
            h
        };
        let uri: Uri = "/v1/runs/r1/kill".parse().unwrap();

        let first =
            mutation_passthrough(State(state.clone()), headers(), uri.clone(), Bytes::new()).await;
        assert!(first.is_ok(), "first call within budget succeeds");

        let second = mutation_passthrough(State(state), headers(), uri, Bytes::new())
            .await
            .expect_err("second call exceeds the per-device budget");
        assert!(matches!(second, ProxyError::RateLimited));
    }
}
