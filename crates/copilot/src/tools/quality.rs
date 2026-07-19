//! Quality-plane read tool, backed by `VerdryxClient` (a read-only SQLite reader
//! over `verdryx.db`). The connection is `!Sync`, so it is opened fresh per call
//! inside `spawn_blocking` from the db path held in [`Clients`], never shared.

use async_trait::async_trait;
use serde_json::{Value, json};

use genaryx_connectors::{VerdryxClient, VerdryxError};

use super::{Clients, Tool, ToolError};

pub(super) fn tools() -> Vec<Box<dyn Tool>> {
    vec![Box::new(QualityLatest)]
}

pub(super) struct QualityLatest;

#[async_trait]
impl Tool for QualityLatest {
    fn name(&self) -> &'static str {
        "quality_latest"
    }
    fn description(&self) -> &'static str {
        "The newest Verdryx evaluation run and its rollup (model, timing, case count, mean score, totals). Use to report agent output quality."
    }
    async fn run(&self, clients: &Clients, _args: &Value) -> Result<Value, ToolError> {
        let db_path = clients
            .verdryx_db
            .clone()
            .ok_or(ToolError::Unavailable("quality_latest"))?;
        let result = tokio::task::spawn_blocking(move || -> Result<Value, VerdryxError> {
            let db = VerdryxClient::open(&db_path)?;
            let latest = db.latest_run()?;
            let summary = match &latest {
                Some(run) => db.run_summary(&run.id)?,
                None => None,
            };
            Ok(json!({ "latest_run": latest, "summary": summary }))
        })
        .await
        .map_err(|e| ToolError::Connector {
            tool: "quality_latest",
            detail: e.to_string(),
        })?
        .map_err(|e| ToolError::Connector {
            tool: "quality_latest",
            detail: e.to_string(),
        })?;
        Ok(result)
    }
}
