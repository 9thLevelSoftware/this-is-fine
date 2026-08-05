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

    /// Soft, operator-actionable I/O pressure (disk full / quota). Prefer this
    /// over a bare `Io` when `ErrorKind::StorageFull` (or platform ENOSPC) is seen.
    #[error("disk full or storage quota exceeded: {0}")]
    DiskFull(String),

    #[error("sqlite error: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("toml error: {0}")]
    Toml(String),

    #[error("{0}")]
    Other(String),
}

impl TifError {
    /// Map raw I/O errors into soft `DiskFull` when the OS reports no space.
    pub fn from_io(err: std::io::Error) -> Self {
        if is_disk_full_io(&err) {
            TifError::DiskFull(err.to_string())
        } else {
            TifError::Io(err)
        }
    }
}

/// True when the I/O error indicates free-space / quota exhaustion.
pub fn is_disk_full_io(err: &std::io::Error) -> bool {
    // Prefer OS codes (portable across MSRV; StorageFull is newer).
    match err.raw_os_error() {
        Some(28) => return true,  // ENOSPC (Unix)
        Some(112) => return true, // ERROR_DISK_FULL (Windows)
        Some(39) => return true,  // ERROR_HANDLE_DISK_FULL (Windows)
        _ => {}
    }
    // String fallbacks when the OS/runtime only provides a message.
    let msg = err.to_string().to_ascii_lowercase();
    msg.contains("no space left")
        || msg.contains("not enough space")
        || msg.contains("disk full")
        || msg.contains("edquot")
        || msg.contains("storage full")
        || msg.contains("os error 28")
        || msg.contains("os error 112")
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
