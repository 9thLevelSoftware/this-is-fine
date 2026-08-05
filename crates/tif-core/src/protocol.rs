//! Versioned JSON CLI protocol for agent adapters.

use serde::{Deserialize, Serialize};

use crate::assess::DamageAssessment;
use crate::fire_level::FireLevel;
use crate::orchestrator::{RunId, RunState};
use crate::policy::ContainmentPolicy;
use crate::scoring::DiffMetrics;
use crate::verify::VerificationReport;

/// Protocol version for adapter ↔ core JSON messages.
pub const PROTOCOL_VERSION: u32 = 1;

/// Envelope for JSON responses.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonResponse<T> {
    pub protocol_version: u32,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<T>,
}

impl<T> JsonResponse<T> {
    pub fn ok(data: T) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            ok: true,
            error: None,
            data: Some(data),
        }
    }

    pub fn err(msg: impl Into<String>) -> JsonResponse<()> {
        JsonResponse {
            protocol_version: PROTOCOL_VERSION,
            ok: false,
            error: Some(msg.into()),
            data: None,
        }
    }
}

/// `tif policy resolve` result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyResolveResult {
    pub policy: ContainmentPolicy,
    pub compact_status: String,
}

/// `tif run begin` result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunBeginResult {
    pub run_id: String,
    pub state: RunState,
    pub policy: Option<ContainmentPolicy>,
    pub compact_status: String,
}

/// `tif run complete` input (also CLI flags).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RunCompleteInput {
    pub run_id: String,
    pub metrics: DiffMetrics,
    #[serde(default)]
    pub verification_passed: Option<bool>,
    #[serde(default)]
    pub auto_firebreak: bool,
}

/// Assessment result payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssessResult {
    pub assessment: DamageAssessment,
}

/// Fire level get/set result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FireLevelResult {
    pub fire_level: FireLevel,
    pub name: String,
    pub initial_selectable: bool,
}

/// Rollback result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RollbackResult {
    pub run_id: String,
    pub restored: bool,
    pub message: String,
}

/// Verify result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifyResult {
    pub report: VerificationReport,
}

/// Helper to print either JSON or human text.
pub fn emit_json<T: Serialize>(value: &T) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(value)
}

/// Parse run id from string.
pub fn parse_run_id(s: &str) -> RunId {
    RunId(s.to_string())
}
