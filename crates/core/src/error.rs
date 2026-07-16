//! Core error type. Fail-closed on the privileged path (06 §0.5): any uncertainty
//! on a command becomes a refusal with a reason, never a silent fall-through.

use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),

    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    /// A line failed conformance validation. Carries the joined validator messages.
    #[error("conformance failed: {0}")]
    Conform(String),

    #[error("store: {0}")]
    Store(String),

    #[error("{0}")]
    Other(String),
}
