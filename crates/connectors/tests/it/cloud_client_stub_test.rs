//! `CloudClient` against a loopback stub Cloud: every read reaches its route
//! with the bearer and parses the wire shape, an absent route is "cannot say"
//! and never an empty answer, every error status is classified as what it
//! is, and every signed mutation carries a signature that verifies over the
//! exact path and body the request carried. The live-Cloud test beside this
//! one skips wherever no TokenFuse checkout exists, so this is the client's
//! own contract held without one.

use genaryx_connectors::{CloudClient, ConnectorError, Severity};
use genaryx_signing::{
    Es256Signer as _, SoftwareSigner, body_sha256_hex, canonical_request, verify_es256_b64,
};

use crate::loopback::{Seen, closed_base, serve};

const RUN: &str = r#"[{"run_id":"r1","model":"m-1","agent_id":"agent://acme.example/bot","spent_microusd":9000,"calls":5,"cache_hits":1,"steps":3,"last_seen_millis":1758000000000,"killed":false,"blocked":2,"last_decision":"budget_exceeded"}]"#;

fn ok(key: &'static str, body: &str) -> (&'static str, &'static str, String) {
    (key, "200 OK", body.to_string())
}

#[tokio::test]
async fn every_read_reaches_its_route_with_the_bearer_and_parses_the_wire_shape() {
    let stub = serve(vec![
        ok(
            "GET /v1/summary",
            r#"{"runs":3,"calls":42,"spent_microusd":150000}"#,
        ),
        ok("GET /v1/runs", RUN),
        ok(
            "GET /v1/agents",
            r#"[{"agent_id":"agent://acme.example/bot","spent_microusd":9000,"calls":5,"runs":1,"last_seen_millis":7}]"#,
        ),
        ok(
            "GET /v1/owners",
            r#"[{"owner":"user://acme.example/pat","spent_microusd":9000,"calls":5,"runs":1,"agents":1,"last_seen_millis":7,"tool_calls":4}]"#,
        ),
        ok(
            "GET /v1/savings",
            r#"{"blocked_spend_microusd":10,"cache_saved_microusd":20,"router_saved_microusd":30,"budget_breaks":2,"total_saved_microusd":60}"#,
        ),
        ok(
            "GET /v1/incidents",
            r#"[{"id":"inc-1","org":"acme","run_id":"r1","agent_id":null,"kind":"budget_exhausted","severity":"high","first_seen_millis":1,"last_seen_millis":2,"occurrences":3,"acknowledged":false,"last_notified_millis":2}]"#,
        ),
        ok(
            "GET /v1/alerts",
            r#"[{"run_id":"r1","spent_microusd":9000,"budget_micros":10000,"fraction":0.9,"killed":false}]"#,
        ),
        ok("GET /v1/budgets", r#"{"r1":4500}"#),
        ok("GET /v1/unit-budgets", r#"{"ops":2000000}"#),
        ok(
            "GET /v1/units",
            r#"[{"unit":"ops","spent_microusd":5,"calls":1,"runs":1,"last_seen_millis":3,"month":"2026-11","month_spent_microusd":5}]"#,
        ),
        ok(
            "GET /v1/gateways",
            r#"[{"site":"edge-1","last_push_millis":99,"pushes":4}]"#,
        ),
        ok(
            "GET /v1/spend",
            r#"{"days_requested":7,"days_covered":3,"from_day":20000,"spent_microusd":9000,"calls":5,"blocked":1,"agents":[{"agent_id":"agent://acme.example/bot","spent_microusd":9000,"calls":5,"blocked":1}]}"#,
        ),
        ok("GET /v1/audit/verify", r#"{"ok":false,"break_index":4}"#),
        ok("GET /v1/compliance/evidence", r#"{"controls":["a","b"]}"#),
    ]);
    // A trailing slash on the base is trimmed, not doubled into every path.
    let c = CloudClient::new(format!("{}/", stub.base), "test-org-key").expect("client");

    let s = c.summary().await.expect("summary");
    assert_eq!((s.runs, s.calls, s.spent_microusd), (3, 42, 150_000));
    assert_eq!(
        s.pii_calls, None,
        "an older Cloud's summary reports no PII figure"
    );

    let runs = c.runs().await.expect("runs");
    assert_eq!(runs[0].run_id, "r1");
    assert_eq!(runs[0].blocked, Some(2));
    assert_eq!(runs[0].last_decision.as_deref(), Some("budget_exceeded"));

    assert_eq!(c.agents().await.expect("agents")[0].runs, 1);
    assert_eq!(c.owners().await.expect("owners")[0].tool_calls, 4);
    assert_eq!(c.savings().await.expect("savings").total_saved_microusd, 60);

    let incidents = c.incidents().await.expect("incidents");
    assert_eq!(incidents[0].severity, Severity::High);
    assert_eq!(incidents[0].agent_id, None);
    assert_eq!(incidents[0].summary, None);

    let alerts = c.alerts().await.expect("alerts");
    assert_eq!(alerts[0].budget_micros, 10_000);
    assert_eq!(c.budgets().await.expect("budgets").get("r1"), Some(&4500));
    assert_eq!(
        c.unit_budgets()
            .await
            .expect("unit budgets")
            .expect("the route exists")
            .get("ops"),
        Some(&2_000_000)
    );
    let units = c.units().await.expect("units").expect("the route exists");
    assert_eq!(units[0].month_spent_microusd, 5);
    assert_eq!(units[0].tool_calls, 0, "an absent column defaults");
    let gateways = c
        .gateways()
        .await
        .expect("gateways")
        .expect("the route exists");
    assert_eq!((gateways[0].pushes, gateways[0].calls), (4, 0));

    let window = c
        .spend_window(7)
        .await
        .expect("spend")
        .expect("the route exists");
    assert_eq!((window.days_covered, window.from_day), (3, Some(20_000)));
    assert_eq!(window.agents[0].blocked, 1);

    let audit = c.audit_verify().await.expect("audit");
    assert!(!audit.ok);
    assert_eq!(audit.break_index, Some(4));
    assert_eq!(
        c.compliance_evidence().await.expect("evidence")["controls"][1],
        "b"
    );

    let seen = stub.requests();
    assert_eq!(seen.len(), 14, "one request per read: {seen:?}");
    for r in &seen {
        assert_eq!(r.method, "GET", "{r:?}");
        assert_eq!(r.header("authorization"), Some("Bearer test-org-key"));
        assert!(!r.target.starts_with("//"), "a doubled slash: {}", r.target);
    }
    assert!(
        seen.iter().any(|r| r.target == "/v1/spend?days=7"),
        "the period is asked for in the query"
    );
}

#[tokio::test]
async fn a_cloud_without_a_route_cannot_say_and_is_never_read_as_empty() {
    let stub = serve(Vec::new());
    let c = CloudClient::new(stub.base.clone(), "k").expect("client");
    assert!(
        c.spend_window(30)
            .await
            .expect("404 is an answer")
            .is_none()
    );
    assert!(c.unit_budgets().await.expect("404 is an answer").is_none());
    assert!(c.units().await.expect("404 is an answer").is_none());
    assert!(c.gateways().await.expect("404 is an answer").is_none());
    // A route every Cloud has is an error when it is missing, not "none".
    match c.summary().await {
        Err(ConnectorError::Api { status, body }) => {
            assert_eq!(status, 404);
            assert!(body.contains("not found"), "{body}");
        }
        other => panic!("expected Api 404, got {other:?}"),
    }
}

#[tokio::test]
async fn error_statuses_are_classified_as_what_they_are() {
    let stub = serve(vec![
        (
            "GET /v1/alerts",
            "402 Payment Required",
            r#"{"error":{"type":"plan_required","feature":"alerts","org":"acme","upgrade_url":"https://example.invalid/plan"}}"#
                .to_string(),
        ),
        (
            "GET /v1/savings",
            "402 Payment Required",
            "a 402 that is not the envelope".to_string(),
        ),
        (
            "GET /v1/runs",
            "403 Forbidden",
            r#"{"error":"admin role required"}"#.to_string(),
        ),
        (
            "GET /v1/agents",
            "403 Forbidden",
            r#"{"error":"signature_invalid"}"#.to_string(),
        ),
        (
            "GET /v1/owners",
            "500 Internal Server Error",
            "boom".to_string(),
        ),
        ok("GET /v1/summary", "this is not json"),
    ]);
    let c = CloudClient::new(stub.base.clone(), "k").expect("client");

    match c.alerts().await {
        Err(ConnectorError::PlanRequired {
            feature,
            org,
            upgrade_url,
        }) => {
            assert_eq!(
                (feature.as_str(), org.as_str(), upgrade_url.as_str()),
                ("alerts", "acme", "https://example.invalid/plan")
            );
        }
        other => panic!("expected PlanRequired, got {other:?}"),
    }
    assert!(matches!(
        c.savings().await,
        Err(ConnectorError::Api { status: 402, .. })
    ));
    match c.runs().await {
        Err(ConnectorError::Api { status, body }) => {
            assert_eq!(status, 403);
            assert!(body.contains("admin role required"));
        }
        other => panic!("a 403 that is not a signature failure stays Api, got {other:?}"),
    }
    assert!(matches!(
        c.agents().await,
        Err(ConnectorError::SignatureRejected)
    ));
    assert!(matches!(
        c.owners().await,
        Err(ConnectorError::Api { status: 500, .. })
    ));
    assert!(matches!(c.summary().await, Err(ConnectorError::Json(_))));

    let gone = CloudClient::new(closed_base(), "k").expect("client");
    assert!(matches!(
        gone.summary().await,
        Err(ConnectorError::Transport(_))
    ));
}

/// The signed-mutation headers of one request, checked against `public_b64`
/// over exactly the path and body the stub received.
fn assert_signed(r: &Seen, public_b64: &str, device: &str, token: &str) {
    assert_eq!(r.method, "POST");
    assert_eq!(r.header("x-fuse-device"), Some(device));
    assert_eq!(
        r.header("authorization"),
        Some(format!("Bearer {token}").as_str())
    );
    let ts = r.header("x-fuse-ts").expect("a timestamp");
    let nonce = r.header("x-fuse-nonce").expect("a nonce");
    let sig = r.header("x-fuse-sig").expect("a signature");
    let canonical = canonical_request(
        "POST",
        &r.target,
        &body_sha256_hex(r.body.as_bytes()),
        ts,
        nonce,
    );
    verify_es256_b64(public_b64, canonical.as_bytes(), sig)
        .expect("the signature verifies over the path and body actually sent");
}

#[tokio::test]
async fn signed_mutations_verify_over_the_encoded_path_and_the_exact_body() {
    let stub = serve(vec![
        ok("POST /v1/runs/run%2F1/kill", r#"{"killed":"run/1"}"#),
        ok(
            "POST /v1/runs/run%2F1/budget",
            r#"{"run":"run/1","budget_micros":250000}"#,
        ),
        ok(
            "POST /v1/units/ops%20team/budget",
            r#"{"unit":"ops team","budget_micros":5000000}"#,
        ),
        ok(
            "POST /v1/incidents/inc-1/ack",
            r#"{"acknowledged":"inc-1"}"#,
        ),
    ]);
    let signer = SoftwareSigner::generate().expect("a software key");
    let public = signer.public_key_b64().expect("public key");
    let mut c = CloudClient::new(stub.base.clone(), "org-key").expect("client");
    assert!(!c.has_device());
    c.attach_device("dev-1", "device-token-not-real", Box::new(signer));
    assert!(c.has_device());

    assert_eq!(c.kill_run("run/1").await.expect("kill").killed, "run/1");
    let b = c.set_budget("run/1", 0.25).await.expect("budget");
    assert_eq!((b.run.as_str(), b.budget_micros), ("run/1", 250_000));
    let u = c
        .set_unit_budget("ops team", 5.0)
        .await
        .expect("unit budget");
    assert_eq!((u.unit.as_str(), u.budget_micros), ("ops team", 5_000_000));
    assert_eq!(
        c.ack_incident("inc-1").await.expect("ack").acknowledged,
        "inc-1"
    );

    let seen = stub.requests();
    assert_eq!(seen.len(), 4);
    for r in &seen {
        assert_signed(r, &public, "dev-1", "device-token-not-real");
    }
    assert_eq!(seen[0].target, "/v1/runs/run%2F1/kill");
    assert_eq!(seen[0].body, "", "a kill carries an empty body");
    assert_eq!(seen[1].body, r#"{"budget_usd":0.25}"#);
    assert_eq!(seen[2].target, "/v1/units/ops%20team/budget");
    assert_eq!(seen[2].body, r#"{"budget_usd":5.0}"#);

    // The client's Debug names the device but never a credential.
    let debug = format!("{c:?}");
    assert!(debug.contains("dev-1"), "{debug}");
    assert!(!debug.contains("org-key") && !debug.contains("device-token-not-real"));
}

#[tokio::test]
async fn a_mutation_fails_closed_before_anything_is_sent() {
    let stub = serve(vec![
        ok("POST /v1/runs/r1/kill", r#"{"killed":"r1"}"#),
        (
            "POST /v1/runs/r2/kill",
            "403 Forbidden",
            r#"{"error":"signature_invalid"}"#.to_string(),
        ),
        (
            "POST /v1/incidents/nope/ack",
            "404 Not Found",
            r#"{"error":"unknown incident"}"#.to_string(),
        ),
    ]);
    let unpaired = CloudClient::new(stub.base.clone(), "k").expect("client");
    assert!(matches!(
        unpaired.kill_run("r1").await,
        Err(ConnectorError::NoDeviceSigner)
    ));
    assert!(matches!(
        unpaired.sign_evidence_manifest(b"{}"),
        Err(ConnectorError::NoDeviceSigner)
    ));
    assert!(stub.requests().is_empty(), "no unsigned POST ever leaves");

    let mut c = CloudClient::new(stub.base.clone(), "k").expect("client");
    c.attach_device(
        "dev-2",
        "t",
        Box::new(SoftwareSigner::generate().expect("key")),
    );
    for bad in ["", ".", ".."] {
        assert!(
            matches!(
                c.set_budget(bad, 1.0).await,
                Err(ConnectorError::InvalidPathSegment(_))
            ),
            "{bad:?} addressed something"
        );
    }
    assert!(
        stub.requests().is_empty(),
        "a bad id never reaches the wire"
    );

    assert!(matches!(
        c.kill_run("r2").await,
        Err(ConnectorError::SignatureRejected)
    ));
    assert!(matches!(
        c.ack_incident("nope").await,
        Err(ConnectorError::Api { status: 404, .. })
    ));
}

#[tokio::test]
async fn pairing_mints_with_the_admin_key_and_redeems_with_the_code_alone() {
    let stub = serve(vec![
        ok(
            "POST /v1/pair/new",
            r#"{"code":"code-123","expires_unix":99}"#,
        ),
        ok(
            "POST /v1/pair",
            r#"{"device_id":"dev-9","org":"acme","role":"admin","device_token":"tok-not-a-real-secret"}"#,
        ),
    ]);
    let signer = SoftwareSigner::generate().expect("key");
    let public = signer.public_key_b64().expect("public key");
    let c = CloudClient::new(stub.base.clone(), "reader-key").expect("client");
    let paired = c.pair("admin-key", &signer).await.expect("paired");
    assert_eq!(
        (paired.device_id.as_str(), paired.role.as_str()),
        ("dev-9", "admin")
    );
    let debug = format!("{paired:?}");
    assert!(
        !debug.contains("tok-not-a-real-secret") && debug.contains("<redacted>"),
        "{debug}"
    );

    let seen = stub.requests();
    assert_eq!(seen.len(), 2);
    assert_eq!(seen[0].target, "/v1/pair/new");
    assert_eq!(seen[0].header("authorization"), Some("Bearer admin-key"));
    assert_eq!(seen[1].target, "/v1/pair");
    assert_eq!(
        seen[1].header("authorization"),
        None,
        "the one-time code is the only credential on the redemption"
    );
    let body: serde_json::Value = serde_json::from_str(&seen[1].body).expect("json body");
    assert_eq!(body["code"], "code-123");
    assert_eq!(body["pubkey_b64"], public);
    assert_eq!(body["platform"], std::env::consts::OS);
    assert_eq!(body["name"], "genaryx-connectors (software-signed)");
}

#[tokio::test]
async fn a_refused_pairing_code_is_an_error_and_nothing_is_redeemed() {
    let stub = serve(vec![(
        "POST /v1/pair/new",
        "403 Forbidden",
        r#"{"error":"admin role required"}"#.to_string(),
    )]);
    let signer = SoftwareSigner::generate().expect("key");
    let c = CloudClient::new(stub.base.clone(), "k").expect("client");
    assert!(matches!(
        c.pair("viewer-key", &signer).await,
        Err(ConnectorError::Api { status: 403, .. })
    ));
    assert_eq!(
        stub.requests().len(),
        1,
        "no redemption after a refused mint"
    );
}

#[test]
fn an_evidence_manifest_is_signed_with_the_attached_device_key() {
    let signer = SoftwareSigner::generate().expect("key");
    let public = signer.public_key_b64().expect("public key");
    let mut c = CloudClient::new("http://127.0.0.1:9", "k").expect("client");
    c.attach_device("dev-3", "t", Box::new(signer));
    let manifest = br#"{"pack_version":"1"}"#;
    let block = c.sign_evidence_manifest(manifest).expect("signed");
    assert_eq!(block.alg, "ES256");
    assert_eq!(block.over, "manifest.json");
    assert_eq!(block.public_key_b64, public);
    verify_es256_b64(&block.public_key_b64, manifest, &block.signature_b64)
        .expect("the block verifies over the manifest bytes");
    assert!(
        verify_es256_b64(
            &block.public_key_b64,
            b"another manifest",
            &block.signature_b64
        )
        .is_err(),
        "and over nothing else"
    );
}

#[tokio::test]
async fn hetzner_lists_servers_with_an_encoded_selector_and_refuses_an_error_status() {
    use genaryx_connectors::{HetznerClient, HetznerError};

    let stub = serve(vec![ok(
        "GET /servers",
        r#"{"servers":[{"id":7,"name":"edge","status":"running","public_net":{"ipv4":{"ip":""}},"server_type":{"name":"cx22","cores":2,"memory":4.0,"prices":[{"location":"fsn1","price_hourly":{"net":"not-a-number"}}]},"datacenter":{"location":{"name":"fsn1"}}}]}"#,
    )]);
    let c = HetznerClient::with_base(format!("{}/", stub.base), "test-api-token").expect("client");
    let servers = c
        .list_servers(Some("managed-by=taipan"))
        .await
        .expect("servers");
    assert_eq!(servers.len(), 1);
    assert_eq!(servers[0].ipv4, None, "an empty address is no address");
    assert_eq!(
        servers[0].price_hourly_eur, None,
        "an unreadable price is no price, never zero"
    );
    assert_eq!(
        (servers[0].cores, servers[0].location.as_str()),
        (2, "fsn1")
    );
    assert!(servers[0].labels.is_empty() && servers[0].created.is_empty());
    c.list_servers(None).await.expect("unfiltered");
    let seen = stub.requests();
    assert_eq!(
        seen[0].target,
        "/servers?label_selector=managed-by%3Dtaipan"
    );
    assert_eq!(seen[1].target, "/servers");
    assert_eq!(
        seen[0].header("authorization"),
        Some("Bearer test-api-token")
    );

    let refusing = serve(vec![(
        "GET /servers",
        "401 Unauthorized",
        r#"{"error":{"code":"unauthorized"}}"#.to_string(),
    )]);
    let c = HetznerClient::with_base(refusing.base.clone(), "t").expect("client");
    match c.list_servers(None).await {
        Err(HetznerError::Api { status, body }) => {
            assert_eq!(status, 401);
            assert!(body.contains("unauthorized"), "{body}");
        }
        other => panic!("expected Api 401, got {other:?}"),
    }
    let garbled = serve(vec![ok("GET /servers", "<html>")]);
    let c = HetznerClient::with_base(garbled.base.clone(), "t").expect("client");
    assert!(matches!(
        c.list_servers(None).await,
        Err(HetznerError::Json(_))
    ));
    let c = HetznerClient::with_base(closed_base(), "t").expect("client");
    assert!(matches!(
        c.list_servers(None).await,
        Err(HetznerError::Transport(_))
    ));
}
