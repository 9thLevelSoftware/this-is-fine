//! Error types for This Is Fine core operations.

use thiserror::Error;

/// Top-level error for tif-core.
#[derive(Debug, Error)]
pub enum TifError {
    #[error("configuration error: {0}")]
    Config(String),

    #[error("schema version {found} is not supported (expected {expected})")]
    UnsupportedSchema { found: u32, expected: u32 },

    #[error("invalid fire level: {0}")]
    InvalidFireLevel(String),

    #[error("five-alarm cannot be selected for initial implementation; it is escalation-only")]
    FiveAlarmInitialForbidden,

    #[error("run not found: {0}")]
    RunNotFound(String),

    #[error("invalid run state transition: {from} -> {to}")]
    InvalidTransition { from: String, to: String },

    #[error("correctness floor failed: {0}")]
    CorrectnessFloor(String),

    #[error("policy hard limit violated: {0}")]
    HardLimit(String),

    #[error("reviewer not authorized: {0}")]
    UnauthorizedReviewer(String),

    #[error("isolation error: {0}")]
    Isolation(String),

    #[error("verification error: {0}")]
    Verification(String),

    #[error("audit/storage error: {0}")]
    Audit(String),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("toml error: {0}")]
    Toml(String),

    #[error("{0}")]
    Other(String),
}

impl From<toml::de::Error> for TifError {
    fn from(value: toml::de::Error) -> Self {
        TifError::Toml(value.to_string())
    }
}

impl From<toml::ser::Error> for TifError {
    fn from(value: toml::ser::Error) -> Self {
        TifError::Toml(value.to_string())
    }
}

/// Convenient result alias.
pub type Result<T> = std::result::Result<T, TifError>;
