//! Audit and retention: SQLite metadata + content-addressed artifacts.

use chrono::{Duration, Utc};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::Write;
use std::path::PathBuf;

use crate::config::{AuditConfig, RepoPaths};
use crate::error::{Result, TifError};
use crate::orchestrator::{RunRecord, RunState};

/// Audit retention tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditTier {
    Metadata,
    Redacted,
    Full,
}

impl AuditTier {
    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "metadata" => Ok(AuditTier::Metadata),
            "redacted" => Ok(AuditTier::Redacted),
            "full" => Ok(AuditTier::Full),
            other => Err(TifError::Config(format!("invalid audit tier: {other}"))),
        }
    }
}

/// Local audit store.
pub struct AuditStore {
    conn: Connection,
    artifacts_dir: PathBuf,
    tier: AuditTier,
    max_age_days: u32,
    max_size_mb: u32,
}

impl AuditStore {
    pub fn open(paths: &RepoPaths, audit: &AuditConfig) -> Result<Self> {
        crate::config::ensure_state_dirs(paths)?;
        let conn = Connection::open(paths.db_path())?;
        let store = Self {
            conn,
            artifacts_dir: paths.artifacts_dir(),
            tier: AuditTier::parse(&audit.tier)?,
            max_age_days: audit.max_age_days,
            max_size_mb: audit.max_size_mb,
        };
        store.init_schema()?;
        Ok(store)
    }

    fn init_schema(&self) -> Result<()> {
        self.conn.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS runs (
                id TEXT PRIMARY KEY,
                state TEXT NOT NULL,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                repo_root TEXT NOT NULL,
                task_digest TEXT,
                agent_id TEXT,
                model_id TEXT,
                fire_level INTEGER,
                task_category TEXT,
                simplicity_score REAL,
                contained INTEGER,
                summary TEXT,
                json_blob TEXT
            );

            CREATE TABLE IF NOT EXISTS artifacts (
                hash TEXT PRIMARY KEY,
                kind TEXT NOT NULL,
                size_bytes INTEGER NOT NULL,
                created_at TEXT NOT NULL,
                run_id TEXT
            );

            CREATE TABLE IF NOT EXISTS events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                run_id TEXT NOT NULL,
                at TEXT NOT NULL,
                message TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS adaptation (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            "#,
        )?;
        Ok(())
    }

    /// Persist a run according to the audit tier.
    pub fn record_run(&self, run: &RunRecord) -> Result<()> {
        let task_digest = run
            .task_text
            .as_ref()
            .map(|t| redact_or_digest(t, self.tier));
        let fire_level = run.policy.as_ref().map(|p| p.fire_level.as_u8() as i64);
        let task_category = run
            .policy
            .as_ref()
            .map(|p| p.task_category.as_str().to_string());
        let score = run.score.as_ref().map(|s| s.score);
        let contained = run.state == RunState::Contained || run.state == RunState::Applied;
        let summary = run
            .assessment
            .as_ref()
            .map(|a| a.summary.clone())
            .unwrap_or_default();

        let json_blob = match self.tier {
            AuditTier::Metadata => None,
            AuditTier::Redacted => {
                let mut clone = run.clone();
                if let Some(ref mut t) = clone.task_text {
                    *t = redact_secrets(t);
                }
                Some(serde_json::to_string(&clone)?)
            }
            AuditTier::Full => Some(serde_json::to_string(run)?),
        };

        self.conn.execute(
            r#"
            INSERT INTO runs (
                id, state, created_at, updated_at, repo_root, task_digest,
                agent_id, model_id, fire_level, task_category, simplicity_score,
                contained, summary, json_blob
            ) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14)
            ON CONFLICT(id) DO UPDATE SET
                state=excluded.state,
                updated_at=excluded.updated_at,
                simplicity_score=excluded.simplicity_score,
                contained=excluded.contained,
                summary=excluded.summary,
                json_blob=excluded.json_blob
            "#,
            params![
                run.id.as_str(),
                run.state.as_str(),
                run.created_at.to_rfc3339(),
                run.updated_at.to_rfc3339(),
                run.repo_root,
                task_digest,
                run.agent_id,
                run.model_id,
                fire_level,
                task_category,
                score,
                contained as i32,
                summary,
                json_blob,
            ],
        )?;

        // Replace event history for this run (avoid duplication on re-record).
        self.conn.execute(
            "DELETE FROM events WHERE run_id=?1",
            params![run.id.as_str()],
        )?;
        for ev in &run.events {
            self.conn.execute(
                "INSERT INTO events (run_id, at, message) VALUES (?1, ?2, ?3)",
                params![run.id.as_str(), Utc::now().to_rfc3339(), ev],
            )?;
        }

        Ok(())
    }

    pub fn get_run(&self, id: &str) -> Result<Option<RunRecord>> {
        let mut stmt = self.conn.prepare(
            "SELECT json_blob, state, repo_root, created_at, updated_at FROM runs WHERE id=?1",
        )?;
        let mut rows = stmt.query(params![id])?;
        if let Some(row) = rows.next()? {
            let json_blob: Option<String> = row.get(0)?;
            if let Some(blob) = json_blob {
                let run: RunRecord = serde_json::from_str(&blob)?;
                return Ok(Some(run));
            }
            // Metadata-only: reconstruct a minimal record
            let state: String = row.get(1)?;
            let repo_root: String = row.get(2)?;
            let mut run = RunRecord::new(repo_root);
            run.id = crate::orchestrator::RunId(id.to_string());
            run.state = parse_state(&state).unwrap_or(RunState::Closed);
            return Ok(Some(run));
        }
        Ok(None)
    }

    pub fn list_runs(&self, limit: usize) -> Result<Vec<RunSummary>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, state, created_at, fire_level, task_category, simplicity_score, summary, contained
             FROM runs ORDER BY created_at DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], |row| {
            Ok(RunSummary {
                id: row.get(0)?,
                state: row.get(1)?,
                created_at: row.get(2)?,
                fire_level: row.get(3)?,
                task_category: row.get(4)?,
                simplicity_score: row.get(5)?,
                summary: row.get(6)?,
                contained: row.get::<_, i32>(7)? != 0,
            })
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// Store content-addressed artifact; returns hash.
    pub fn store_artifact(&self, kind: &str, bytes: &[u8], run_id: Option<&str>) -> Result<String> {
        let filtered = match self.tier {
            AuditTier::Metadata => return Ok(String::new()),
            AuditTier::Redacted => redact_secrets(&String::from_utf8_lossy(bytes)).into_bytes(),
            AuditTier::Full => bytes.to_vec(),
        };

        let hash = hex_sha256(&filtered);
        let path = self.artifact_path(&hash);
        if !path.exists() {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut f = fs::File::create(&path)?;
            f.write_all(&filtered)?;
        }
        self.conn.execute(
            "INSERT OR IGNORE INTO artifacts (hash, kind, size_bytes, created_at, run_id) VALUES (?1,?2,?3,?4,?5)",
            params![
                hash,
                kind,
                filtered.len() as i64,
                Utc::now().to_rfc3339(),
                run_id
            ],
        )?;
        Ok(hash)
    }

    fn artifact_path(&self, hash: &str) -> PathBuf {
        let prefix = if hash.len() >= 2 { &hash[..2] } else { "00" };
        self.artifacts_dir.join(prefix).join(hash)
    }

    /// Evict old records and unreferenced artifacts.
    pub fn gc(&self) -> Result<GcReport> {
        let cutoff = Utc::now() - Duration::days(self.max_age_days as i64);
        let deleted_runs = self.conn.execute(
            "DELETE FROM runs WHERE created_at < ?1",
            params![cutoff.to_rfc3339()],
        )?;
        let deleted_events = self.conn.execute(
            "DELETE FROM events WHERE at < ?1",
            params![cutoff.to_rfc3339()],
        )?;

        // Size-based: list artifacts oldest first and delete until under budget.
        let max_bytes = (self.max_size_mb as u64) * 1024 * 1024;
        let total = self.total_artifact_bytes()?;
        let mut reclaimed = 0u64;
        if total > max_bytes {
            let mut stmt = self
                .conn
                .prepare("SELECT hash, size_bytes FROM artifacts ORDER BY created_at ASC")?;
            let rows: Vec<(String, i64)> = stmt
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
                .filter_map(|r| r.ok())
                .collect();
            let mut current = total;
            for (hash, size) in rows {
                if current <= max_bytes {
                    break;
                }
                let path = self.artifact_path(&hash);
                if path.exists() {
                    let _ = fs::remove_file(path);
                }
                self.conn
                    .execute("DELETE FROM artifacts WHERE hash=?1", params![hash])?;
                current = current.saturating_sub(size as u64);
                reclaimed += size as u64;
            }
        }

        Ok(GcReport {
            deleted_runs,
            deleted_events,
            reclaimed_bytes: reclaimed,
        })
    }

    fn total_artifact_bytes(&self) -> Result<u64> {
        let sum: i64 = self.conn.query_row(
            "SELECT COALESCE(SUM(size_bytes),0) FROM artifacts",
            [],
            |r| r.get(0),
        )?;
        Ok(sum as u64)
    }

    pub fn purge_all(&self) -> Result<()> {
        self.conn.execute_batch(
            "DELETE FROM runs; DELETE FROM events; DELETE FROM artifacts; DELETE FROM adaptation;",
        )?;
        if self.artifacts_dir.exists() {
            fs::remove_dir_all(&self.artifacts_dir)?;
            fs::create_dir_all(&self.artifacts_dir)?;
        }
        Ok(())
    }

    /// Number of audit events stored for a run (used by tests and diagnostics).
    pub fn event_count(&self, run_id: &str) -> Result<usize> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM events WHERE run_id=?1",
            params![run_id],
            |r| r.get(0),
        )?;
        Ok(n as usize)
    }

    /// Load local adaptation stats (empty when none recorded).
    pub fn load_adaptation_stats(&self) -> Result<crate::adaptation::AdaptationStats> {
        let mut stmt = self
            .conn
            .prepare("SELECT value FROM adaptation WHERE key='stats'")?;
        let mut rows = stmt.query([])?;
        if let Some(row) = rows.next()? {
            let value: String = row.get(0)?;
            let stats = serde_json::from_str(&value).unwrap_or_default();
            return Ok(stats);
        }
        Ok(crate::adaptation::AdaptationStats::default())
    }

    /// Persist local adaptation stats.
    pub fn save_adaptation_stats(&self, stats: &crate::adaptation::AdaptationStats) -> Result<()> {
        let value = serde_json::to_string(stats)?;
        self.conn.execute(
            "INSERT INTO adaptation (key, value) VALUES ('stats', ?1)
             ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![value],
        )?;
        Ok(())
    }

    /// Record adaptation outcome from a finished run.
    pub fn record_adaptation_from_run(&self, run: &RunRecord) -> Result<()> {
        let mut eng = crate::adaptation::AdaptationEngine::load(self.load_adaptation_stats()?);
        let policy = run.policy.as_ref();
        let fb = run.firebreak.as_ref();
        let contained = matches!(run.state, RunState::Contained | RunState::Applied)
            || run
                .assessment
                .as_ref()
                .is_some_and(|a| a.status == crate::assess::AssessmentStatus::Contained);
        let out_of_control = run
            .assessment
            .as_ref()
            .is_some_and(|a| a.status == crate::assess::AssessmentStatus::OutOfControl)
            || run.state == RunState::OutOfControl
            || run.state == RunState::AwaitingApproval;
        let firebreak =
            fb.map(|f| f.applied || (f.success && f.candidate_ready && !f.requires_approval));
        // Applied success counts as firebreak success; explicit fail/not-ready as fail.
        let firebreak = match (fb, firebreak) {
            (Some(f), _) if f.applied => Some(true),
            (Some(f), _) if f.requires_approval && f.candidate_ready => None, // pending
            (Some(f), _) if !f.success || (!f.applied && !f.candidate_ready) => Some(false),
            (Some(_), Some(v)) => Some(v),
            _ => None,
        };
        let outcome = crate::adaptation::RunOutcome {
            contained,
            out_of_control,
            firebreak,
            rolled_back: run.state == RunState::Restored
                && run.isolation_session.as_ref().is_some_and(|s| !s.applied)
                && run.events.iter().any(|e| e.contains("rollback")),
            category: policy
                .map(|p| p.task_category)
                .unwrap_or(crate::task::TaskCategory::Unknown),
            fire_level: policy
                .map(|p| p.fire_level)
                .unwrap_or(crate::fire_level::FireLevel::Containment),
            simplicity_score: run.score.as_ref().map(|s| s.score).unwrap_or(0.0),
            verification_passed: run
                .verification
                .as_ref()
                .map(|v| v.satisfies_correctness_verification())
                .unwrap_or(false),
            pressure_template_id: policy
                .map(|p| p.pressure.template_id.as_str())
                .unwrap_or("unknown"),
            reviewer_id: fb.and_then(|f| f.reviewer_id.as_deref()),
        };
        eng.record_run(outcome);
        self.save_adaptation_stats(eng.stats())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunSummary {
    pub id: String,
    pub state: String,
    pub created_at: String,
    pub fire_level: Option<i64>,
    pub task_category: Option<String>,
    pub simplicity_score: Option<f64>,
    pub summary: String,
    pub contained: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GcReport {
    pub deleted_runs: usize,
    pub deleted_events: usize,
    pub reclaimed_bytes: u64,
}

fn hex_sha256(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn redact_or_digest(text: &str, tier: AuditTier) -> String {
    match tier {
        AuditTier::Metadata => {
            let h = hex_sha256(text.as_bytes());
            format!("sha256:{}", &h[..16])
        }
        AuditTier::Redacted => redact_secrets(text),
        AuditTier::Full => text.to_string(),
    }
}

/// Redact common secret patterns before persistence.
pub fn redact_secrets(input: &str) -> String {
    // Multi-line PEM private keys first.
    let mut out = redact_pem_blocks(input);

    let mut redacted_lines = Vec::new();
    for line in out.lines() {
        redacted_lines.push(redact_secret_line(line));
    }
    out = redacted_lines.join("\n");
    if input.ends_with('\n') && !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

fn redact_pem_blocks(input: &str) -> String {
    let begin = "-----BEGIN ";
    let end_prefix = "-----END ";
    let mut out = String::new();
    let mut rest = input;
    while let Some(start) = rest.find(begin) {
        out.push_str(&rest[..start]);
        let after_begin = &rest[start..];
        // Only redact private key PEMs.
        let header_line_end = after_begin.find('\n').unwrap_or(after_begin.len());
        let header = &after_begin[..header_line_end];
        if !header.to_ascii_uppercase().contains("PRIVATE KEY") {
            out.push_str(header);
            rest = &after_begin[header_line_end..];
            continue;
        }
        if let Some(end_rel) = after_begin.find(end_prefix) {
            let after_end = &after_begin[end_rel + end_prefix.len()..];
            let end_line = after_end.find('\n').unwrap_or(after_end.len());
            let end = end_rel + end_prefix.len() + end_line;
            let end = if after_begin.get(end..end + 1) == Some("\n") {
                end + 1
            } else {
                end
            };
            out.push_str("[REDACTED PRIVATE KEY]\n");
            rest = &after_begin[end..];
        } else {
            out.push_str("[REDACTED PRIVATE KEY]");
            rest = "";
            break;
        }
    }
    out.push_str(rest);
    out
}

fn redact_secret_line(line: &str) -> String {
    if line.contains("[REDACTED") {
        return line.to_string();
    }
    let lower = line.to_ascii_lowercase();
    let sensitive = lower.contains("api_key")
        || lower.contains("apikey")
        || lower.contains("password")
        || lower.contains("secret")
        || looks_like_token_secret(&lower)
        || lower.contains("bearer ")
        || lower.contains("authorization")
        || lower.contains("private key");
    if !sensitive {
        return line.to_string();
    }

    // authorization: bearer <token>
    if lower.contains("bearer ") {
        if let Some(idx) = lower.find("bearer ") {
            return format!("{}***", &line[..idx + "bearer ".len()]);
        }
    }

    if let Some(idx) = line.find('=') {
        // Exclusive prefix: keep key, single '=' separator.
        return format!("{}=***", &line[..idx]);
    }
    if let Some(idx) = line.find(':') {
        return format!("{}: ***", &line[..idx]);
    }
    "[REDACTED]".into()
}

/// Match secret-like token **keys**, not bare substring "token" (avoids "tokenize", "token bucket").
fn looks_like_token_secret(lower: &str) -> bool {
    lower.contains("token=")
        || lower.contains("token =")
        || lower.contains("token:")
        || lower.contains("token :")
        || lower.contains("access_token")
        || lower.contains("refresh_token")
        || lower.contains("id_token")
        || lower.contains("api_token")
        || lower.contains("auth_token")
        || lower.contains("_token=")
        || lower.contains("_token:")
        || lower.contains("_token =")
        || lower.contains("_token :")
        // Whole-key forms: "token" as sole key before =/:
        || lower.starts_with("token=")
        || lower.starts_with("token:")
        || lower.starts_with("token =")
        || lower.starts_with("token :")
}

fn parse_state(s: &str) -> Option<RunState> {
    match s {
        "preflight" => Some(RunState::Preflight),
        "policy_selected" => Some(RunState::PolicySelected),
        "agent_injected" => Some(RunState::AgentInjected),
        "implementing" => Some(RunState::Implementing),
        "implementation_complete" => Some(RunState::ImplementationComplete),
        "diff_captured" => Some(RunState::DiffCaptured),
        "verifying" => Some(RunState::Verifying),
        "scoring" => Some(RunState::Scoring),
        "contained" => Some(RunState::Contained),
        "out_of_control" => Some(RunState::OutOfControl),
        "firebreak_running" => Some(RunState::FirebreakRunning),
        "candidate_comparing" => Some(RunState::CandidateComparing),
        "awaiting_approval" => Some(RunState::AwaitingApproval),
        "applied" => Some(RunState::Applied),
        "restored" => Some(RunState::Restored),
        "rejected" => Some(RunState::Rejected),
        "closed" => Some(RunState::Closed),
        "failed" => Some(RunState::Failed),
        _ => None,
    }
}

/// Compact status line for agent UIs.
pub fn compact_status(fire_level: u8, active: bool) -> String {
    if active {
        format!("🔥 Containment active · Fire Level {fire_level}")
    } else {
        format!("Containment suspended · Fire Level {fire_level}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AuditConfig, Config};
    use crate::orchestrator::{BeginRunRequest, RunOrchestrator};
    use tempfile::tempdir;

    #[test]
    fn record_and_list_runs() {
        let dir = tempdir().unwrap();
        let paths = RepoPaths::for_root(dir.path());
        crate::config::ensure_state_dirs(&paths).unwrap();
        let store = AuditStore::open(
            &paths,
            &AuditConfig {
                tier: "redacted".into(),
                max_age_days: 90,
                max_size_mb: 100,
            },
        )
        .unwrap();

        let cfg = Config::default();
        let run = RunOrchestrator::new()
            .begin(
                &cfg,
                dir.path().to_string_lossy().as_ref(),
                BeginRunRequest {
                    task_text: Some("fix bug password=supersecret".into()),
                    ..Default::default()
                },
            )
            .unwrap();
        store.record_run(&run).unwrap();
        let list = store.list_runs(10).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, run.id.as_str());
    }

    #[test]
    fn redacts_secrets() {
        let s = redact_secrets("password=hunter2\nnormal line");
        assert!(s.contains("password=***"));
        assert!(!s.contains("hunter2"));
        assert!(!s.contains("=="));
    }

    #[test]
    fn redacts_without_doubled_separators() {
        assert_eq!(redact_secrets("password=hunter2"), "password=***");
        assert_eq!(redact_secrets("api_key: supersecret"), "api_key: ***");
        assert!(redact_secrets("Authorization: Bearer abc.def.ghi").contains("Bearer ***"));
        assert_eq!(redact_secrets("access_token=abc"), "access_token=***");
        assert_eq!(redact_secrets("token=secret"), "token=***");
    }

    #[test]
    fn does_not_over_redact_token_substrings() {
        // Benign uses of the word "token" / "tokenize" must stay intact.
        assert_eq!(
            redact_secrets("use a token bucket for rate limiting"),
            "use a token bucket for rate limiting"
        );
        assert_eq!(
            redact_secrets("tokenizer splits the input stream"),
            "tokenizer splits the input stream"
        );
    }

    #[test]
    fn redacts_pem_private_key_block() {
        let pem =
            "-----BEGIN RSA PRIVATE KEY-----\nMIIEowIBAAKCAQEA\n-----END RSA PRIVATE KEY-----\n";
        let s = redact_secrets(pem);
        assert!(s.contains("[REDACTED PRIVATE KEY]"));
        assert!(!s.contains("MIIEowIBAAKCAQEA"));
    }

    #[test]
    fn re_record_does_not_duplicate_events() {
        let dir = tempdir().unwrap();
        let paths = RepoPaths::for_root(dir.path());
        crate::config::ensure_state_dirs(&paths).unwrap();
        let store = AuditStore::open(
            &paths,
            &AuditConfig {
                tier: "redacted".into(),
                max_age_days: 90,
                max_size_mb: 100,
            },
        )
        .unwrap();
        let cfg = Config::default();
        let mut run = RunOrchestrator::new()
            .begin(
                &cfg,
                dir.path().to_string_lossy().as_ref(),
                BeginRunRequest {
                    task_text: Some("fix bug".into()),
                    ..Default::default()
                },
            )
            .unwrap();
        store.record_run(&run).unwrap();
        store.record_run(&run).unwrap();
        assert_eq!(
            store.event_count(run.id.as_str()).unwrap(),
            run.events.len()
        );
        run.events.push("extra".into());
        store.record_run(&run).unwrap();
        assert_eq!(
            store.event_count(run.id.as_str()).unwrap(),
            run.events.len()
        );
    }
}
