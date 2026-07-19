//! Crypto-plane read tool, backed by the `qryx` CLI (post-quantum readiness).
//! The FIRST parameterized tool: it takes a filesystem `path` to scan. qryx is
//! shelled fresh per call inside `spawn_blocking` from the binary path held in
//! [`Clients`]; a bad path is returned as error-DATA (so the model can correct
//! it), not a hard failure of the whole answer.

use std::path::PathBuf;

use async_trait::async_trait;
use serde_json::{Value, json};

use genaryx_connectors::QryxClient;

use super::{Clients, Tool, ToolError, to_result};

pub(super) fn tools() -> Vec<Box<dyn Tool>> {
    vec![Box::new(CryptoScan)]
}

pub(super) struct CryptoScan;

#[async_trait]
impl Tool for CryptoScan {
    fn name(&self) -> &'static str {
        "crypto_scan"
    }
    fn description(&self) -> &'static str {
        "Scan a filesystem path for post-quantum cryptography readiness (the NCSC 2028/2031/2035 migration timeline: discovery, priorities, findings). Use for crypto-posture questions about a codebase path."
    }
    fn params_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "path": {"type": "string", "description": "absolute filesystem path to scan"}
            },
            "required": ["path"]
        })
    }
    async fn run(&self, clients: &Clients, args: &Value) -> Result<Value, ToolError> {
        let bin = clients
            .qryx_bin
            .clone()
            .ok_or(ToolError::Unavailable("crypto_scan"))?;
        let path_str = args
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| ToolError::BadArgs {
                tool: "crypto_scan",
                detail: "`path` (string) is required".to_string(),
            })?
            .to_string();
        let path = PathBuf::from(&path_str);
        if !path.exists() {
            // Error-as-data: let the model correct the path, don't fail the answer.
            return Ok(json!({ "error": format!("path does not exist: {path_str}") }));
        }
        let report = tokio::task::spawn_blocking(move || {
            let qryx = QryxClient::new(bin);
            qryx.scan_ncsc(&path)
        })
        .await
        .map_err(|e| ToolError::Connector {
            tool: "crypto_scan",
            detail: e.to_string(),
        })?
        .map_err(|e| ToolError::Connector {
            tool: "crypto_scan",
            detail: e.to_string(),
        })?;
        to_result("crypto_scan", report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clients_with_qryx() -> Clients {
        Clients {
            qryx_bin: Some(PathBuf::from("/x/qryx-not-real")),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn missing_path_arg_is_bad_args() {
        let err = CryptoScan
            .run(&clients_with_qryx(), &json!({}))
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            ToolError::BadArgs {
                tool: "crypto_scan",
                ..
            }
        ));
    }

    #[tokio::test]
    async fn nonexistent_path_is_error_as_data_not_a_failure() {
        // A bad path returns Ok(error-json) so the model can correct it; qryx is
        // never reached, so the fake bin path is irrelevant here.
        let out = CryptoScan
            .run(&clients_with_qryx(), &json!({"path": "/no/such/path/xyz"}))
            .await
            .unwrap();
        assert!(out.get("error").is_some());
    }

    #[tokio::test]
    async fn unavailable_when_no_qryx_configured() {
        let err = CryptoScan
            .run(&Clients::default(), &json!({"path": "/tmp"}))
            .await
            .unwrap_err();
        assert!(matches!(err, ToolError::Unavailable("crypto_scan")));
    }

    // Live skip-graceful: if the stack's qryx binary is installed, a scan of a
    // real path succeeds; otherwise skip (this box may not have it).
    #[tokio::test]
    async fn live_crypto_scan_when_qryx_is_installed() {
        let home = std::env::var("HOME").unwrap_or_default();
        let bin = PathBuf::from(format!("{home}/.taipan/bin/qryx"));
        if !bin.exists() {
            eprintln!("SKIP live crypto_scan: no qryx at {}", bin.display());
            return;
        }
        let clients = Clients {
            qryx_bin: Some(bin),
            ..Default::default()
        };
        let here = env!("CARGO_MANIFEST_DIR");
        let out = CryptoScan.run(&clients, &json!({"path": here})).await;
        assert!(
            out.is_ok(),
            "qryx is installed but the scan errored: {out:?}"
        );
        eprintln!("live crypto_scan OK against {here}");
    }
}
