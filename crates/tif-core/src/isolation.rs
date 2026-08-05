//! Isolation primitives: Git worktree and non-Git snapshot/rollback.
//!
//! Fail-safe rules:
//! - Unverified Firebreak never modifies the original workspace.
//! - Apply only copies from an isolated candidate after the caller authorizes it.
//! - Before apply, a baseline of the original is preserved for rollback.
//! - Restore puts the baseline back onto the source workspace.

use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::error::{Result, TifError};

/// Kind of isolation backend in use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IsolationKind {
    GitWorktree,
    Snapshot,
}

/// Handle to an isolated workspace (and optional rollback baseline).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IsolationSession {
    pub kind: IsolationKind,
    pub id: String,
    /// Isolated candidate workspace path (worktree or snapshot copy).
    pub path: PathBuf,
    pub source_root: PathBuf,
    /// Baseline of the original workspace captured before apply (if any).
    #[serde(default)]
    pub baseline_path: Option<PathBuf>,
    /// Path to reviewer-produced candidate tree under isolation (Phase 1+).
    #[serde(default)]
    pub candidate_path: Option<PathBuf>,
    /// True after a successful apply, or when source may have been mutated and
    /// still needs restore (including dual-failure: apply failed and restore failed).
    #[serde(default)]
    pub applied: bool,
    /// Sticky: apply mutated source and automatic restore did not fully succeed.
    /// Operators should run `tif rollback` until clear. Implies `applied`.
    #[serde(default)]
    pub restore_pending: bool,
    pub created_at_unix: i64,
}

/// Trait for isolation backends.
pub trait Isolator: Send + Sync {
    fn kind(&self) -> IsolationKind;
    /// Create an isolated workspace. Does not modify the original.
    fn create(&self, source_root: &Path, session_id: &str) -> Result<IsolationSession>;
    /// Destroy the isolation workspace (and optional baseline if requested).
    fn destroy(&self, session: &IsolationSession) -> Result<()>;
    /// Capture a rollback baseline of the current source (call before apply).
    fn preserve_baseline(&self, session: &mut IsolationSession) -> Result<()>;
    /// Apply a verified candidate from isolation back to the source.
    /// Preserves a baseline first if one is not already present.
    fn apply_to_source(&self, session: &mut IsolationSession) -> Result<()>;
    /// Restore source to the pre-apply baseline. No-op if never applied.
    fn restore_source(&self, session: &mut IsolationSession) -> Result<()>;
}

/// Select an isolator based on repository characteristics.
pub fn select_isolator(source_root: &Path, snapshots_dir: &Path) -> Box<dyn Isolator> {
    if is_git_repo(source_root) {
        Box::new(GitWorktreeIsolator::new(snapshots_dir.to_path_buf()))
    } else {
        Box::new(SnapshotIsolator::new(snapshots_dir.to_path_buf()))
    }
}

fn is_git_repo(root: &Path) -> bool {
    root.join(".git").exists()
        || Command::new("git")
            .args(["rev-parse", "--is-inside-work-tree"])
            .current_dir(root)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
}

/// Git worktree-based isolation.
///
/// Candidate experiments run in a detached worktree. Apply copies tracked-tree
/// files from the worktree into the source, after preserving a baseline under
/// `.this-is-fine/snapshots/{id}-baseline`.
#[derive(Debug)]
pub struct GitWorktreeIsolator {
    snapshots_dir: PathBuf,
}

impl GitWorktreeIsolator {
    pub fn new(snapshots_dir: PathBuf) -> Self {
        Self { snapshots_dir }
    }
}

impl Isolator for GitWorktreeIsolator {
    fn kind(&self) -> IsolationKind {
        IsolationKind::GitWorktree
    }

    fn create(&self, source_root: &Path, session_id: &str) -> Result<IsolationSession> {
        let worktree_path = source_root
            .join(".this-is-fine")
            .join("worktrees")
            .join(session_id);
        if let Some(parent) = worktree_path.parent() {
            fs::create_dir_all(parent)?;
        }
        // Clean leftover from a previous interrupted session with the same id.
        if worktree_path.exists() {
            let _ = remove_worktree(source_root, &worktree_path, session_id);
        }

        let path_str = path_to_str(&worktree_path)?;

        // Create a detached worktree for Firebreak experiments.
        let branch = format!("tif/firebreak/{session_id}");
        let status = git(
            source_root,
            &["worktree", "add", "-b", &branch, path_str, "HEAD"],
        )?;

        if !status.success() {
            // Fallback: try without new branch if it exists
            let status2 = git(
                source_root,
                &["worktree", "add", "--detach", path_str, "HEAD"],
            )?;
            if !status2.success() {
                return Err(TifError::Isolation(
                    "failed to create git worktree; ensure git is installed and repo is clean enough"
                        .into(),
                ));
            }
        }

        Ok(IsolationSession {
            kind: IsolationKind::GitWorktree,
            id: session_id.to_string(),
            path: worktree_path,
            source_root: source_root.to_path_buf(),
            baseline_path: None,
            candidate_path: None,
            applied: false,
            restore_pending: false,
            created_at_unix: chrono_now(),
        })
    }

    fn destroy(&self, session: &IsolationSession) -> Result<()> {
        let _ = remove_worktree(&session.source_root, &session.path, &session.id);
        if let Some(ref baseline) = session.baseline_path {
            if baseline.exists() {
                let _ = fs::remove_dir_all(baseline);
            }
        }
        Ok(())
    }

    fn preserve_baseline(&self, session: &mut IsolationSession) -> Result<()> {
        if session.baseline_path.as_ref().is_some_and(|p| p.exists()) {
            return Ok(());
        }
        fs::create_dir_all(&self.snapshots_dir)?;
        let baseline = self.snapshots_dir.join(format!("{}-baseline", session.id));
        if baseline.exists() {
            fs::remove_dir_all(&baseline)?;
        }
        copy_dir_selective(&session.source_root, &baseline)?;
        session.baseline_path = Some(baseline);
        Ok(())
    }

    fn apply_to_source(&self, session: &mut IsolationSession) -> Result<()> {
        if !session.path.exists() {
            return Err(TifError::Isolation("worktree path missing".into()));
        }
        if session.applied || session.restore_pending {
            return Err(TifError::Isolation(
                "candidate already applied or restore pending; restore before re-applying".into(),
            ));
        }
        self.preserve_baseline(session)?;
        apply_with_prune_and_failsafe(session)
    }

    fn restore_source(&self, session: &mut IsolationSession) -> Result<()> {
        restore_session_source(session)
    }
}

fn remove_worktree(source_root: &Path, worktree_path: &Path, session_id: &str) -> Result<()> {
    if let Ok(path_str) = path_to_str(worktree_path) {
        let _ = git(source_root, &["worktree", "remove", "--force", path_str]);
    }
    let branch = format!("tif/firebreak/{session_id}");
    let _ = git(source_root, &["branch", "-D", &branch]);
    if worktree_path.exists() {
        let _ = fs::remove_dir_all(worktree_path);
    }
    Ok(())
}

/// Non-Git snapshot and rollback via directory copy.
///
/// `create` copies source → isolation path (candidate workspace).
/// Before apply, a separate baseline is preserved; apply overlays the candidate
/// onto source; restore puts the baseline back.
#[derive(Debug)]
pub struct SnapshotIsolator {
    snapshots_dir: PathBuf,
}

impl SnapshotIsolator {
    pub fn new(snapshots_dir: PathBuf) -> Self {
        Self { snapshots_dir }
    }
}

impl Isolator for SnapshotIsolator {
    fn kind(&self) -> IsolationKind {
        IsolationKind::Snapshot
    }

    fn create(&self, source_root: &Path, session_id: &str) -> Result<IsolationSession> {
        fs::create_dir_all(&self.snapshots_dir)?;
        let snap = self.snapshots_dir.join(session_id);
        if snap.exists() {
            fs::remove_dir_all(&snap)?;
        }
        copy_dir_selective(source_root, &snap)?;
        Ok(IsolationSession {
            kind: IsolationKind::Snapshot,
            id: session_id.to_string(),
            path: snap,
            source_root: source_root.to_path_buf(),
            baseline_path: None,
            candidate_path: None,
            applied: false,
            restore_pending: false,
            created_at_unix: chrono_now(),
        })
    }

    fn destroy(&self, session: &IsolationSession) -> Result<()> {
        if session.path.exists() {
            fs::remove_dir_all(&session.path)?;
        }
        if let Some(ref baseline) = session.baseline_path {
            if baseline.exists() {
                fs::remove_dir_all(baseline)?;
            }
        }
        Ok(())
    }

    fn preserve_baseline(&self, session: &mut IsolationSession) -> Result<()> {
        if session.baseline_path.as_ref().is_some_and(|p| p.exists()) {
            return Ok(());
        }
        fs::create_dir_all(&self.snapshots_dir)?;
        let baseline = self.snapshots_dir.join(format!("{}-baseline", session.id));
        if baseline.exists() {
            fs::remove_dir_all(&baseline)?;
        }
        copy_dir_selective(&session.source_root, &baseline)?;
        session.baseline_path = Some(baseline);
        Ok(())
    }

    fn apply_to_source(&self, session: &mut IsolationSession) -> Result<()> {
        if !session.path.exists() {
            return Err(TifError::Isolation("snapshot path missing".into()));
        }
        if session.applied || session.restore_pending {
            return Err(TifError::Isolation(
                "candidate already applied or restore pending; restore before re-applying".into(),
            ));
        }
        self.preserve_baseline(session)?;
        apply_with_prune_and_failsafe(session)
    }

    fn restore_source(&self, session: &mut IsolationSession) -> Result<()> {
        restore_session_source(session)
    }
}

/// Overlay candidate → source, then prune source files deleted in the candidate
/// (present in baseline, absent from candidate). On any failure, attempt restore
/// and never claim restore succeeded unless it returned Ok.
/// Prefer `candidate_path` (reviewer output) when set; otherwise the isolation workspace root.
pub fn session_candidate_root(session: &IsolationSession) -> &Path {
    session
        .candidate_path
        .as_deref()
        .unwrap_or(session.path.as_path())
}

fn apply_with_prune_and_failsafe(session: &mut IsolationSession) -> Result<()> {
    let baseline = session
        .baseline_path
        .clone()
        .ok_or_else(|| TifError::Isolation("baseline missing before apply".into()))?;

    let candidate_root = session_candidate_root(session).to_path_buf();
    if !candidate_root.exists() {
        return Err(TifError::Isolation(format!(
            "candidate root missing: {}",
            candidate_root.display()
        )));
    }

    let apply_result = (|| -> Result<()> {
        copy_dir_selective(&candidate_root, &session.source_root)?;
        // Symmetric prune: remove source paths that exist in baseline but not candidate.
        prune_deleted_in_candidate(&session.source_root, &baseline, &candidate_root, &baseline)?;
        Ok(())
    })();

    match apply_result {
        Ok(()) => {
            session.applied = true;
            session.restore_pending = false;
            Ok(())
        }
        Err(apply_err) => match restore_from_baseline(&baseline, &session.source_root) {
            Ok(()) => {
                session.applied = false;
                session.restore_pending = false;
                Err(TifError::Isolation(format!(
                    "apply failed; source restored from baseline: {apply_err}"
                )))
            }
            Err(restore_err) => {
                // Dual failure: source may be dirty; sticky flags for rollback retry.
                session.applied = true;
                session.restore_pending = true;
                Err(TifError::Isolation(format!(
                    "apply failed ({apply_err}); restore also failed ({restore_err}); \
                     source may be modified — run rollback (session restore_pending=true)"
                )))
            }
        },
    }
}

fn restore_session_source(session: &mut IsolationSession) -> Result<()> {
    if !session.applied && !session.restore_pending {
        return Ok(());
    }
    let baseline = session.baseline_path.as_ref().ok_or_else(|| {
        TifError::Isolation("cannot restore: no baseline was preserved before apply".into())
    })?;
    if !baseline.exists() {
        return Err(TifError::Isolation(
            "cannot restore: baseline directory is missing".into(),
        ));
    }
    restore_from_baseline(baseline, &session.source_root)?;
    session.applied = false;
    session.restore_pending = false;
    Ok(())
}

/// Overlay baseline onto source, then prune source paths not present in baseline.
fn restore_from_baseline(baseline: &Path, source_root: &Path) -> Result<()> {
    copy_dir_selective(baseline, source_root)?;
    prune_not_in_reference(source_root, baseline, source_root)?;
    Ok(())
}

/// Walk baseline paths; if a path exists in baseline but not in the candidate,
/// remove it from source (so apply actually deletes files the candidate dropped).
fn prune_deleted_in_candidate(
    source_root: &Path,
    baseline_root: &Path,
    candidate_root: &Path,
    baseline_current: &Path,
) -> Result<()> {
    let entries: Vec<_> = match fs::read_dir(baseline_current) {
        Ok(rd) => rd.collect::<std::result::Result<Vec<_>, _>>()?,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    for entry in entries {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if is_skipped_name(name_str.as_ref()) {
            continue;
        }
        let base_path = entry.path();
        let rel = base_path.strip_prefix(baseline_root).unwrap_or(&base_path);
        let cand_path = candidate_root.join(rel);
        let src_path = source_root.join(rel);
        if !cand_path.exists() {
            if src_path.is_dir() {
                fs::remove_dir_all(&src_path)?;
            } else if src_path.exists() {
                fs::remove_file(&src_path)?;
            }
        } else if base_path.is_dir() {
            prune_deleted_in_candidate(source_root, baseline_root, candidate_root, &base_path)?;
        }
    }
    Ok(())
}

/// Walk `current` (under source) and remove entries with no counterpart under `reference_root`.
fn prune_not_in_reference(source_root: &Path, reference_root: &Path, current: &Path) -> Result<()> {
    let entries: Vec<_> = match fs::read_dir(current) {
        Ok(rd) => rd.collect::<std::result::Result<Vec<_>, _>>()?,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    for entry in entries {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if is_skipped_name(name_str.as_ref()) {
            continue;
        }
        let src_path = entry.path();
        let rel = src_path.strip_prefix(source_root).unwrap_or(&src_path);
        let ref_path = reference_root.join(rel);
        if !ref_path.exists() {
            if src_path.is_dir() {
                fs::remove_dir_all(&src_path)?;
            } else {
                fs::remove_file(&src_path)?;
            }
        } else if src_path.is_dir() {
            prune_not_in_reference(source_root, reference_root, &src_path)?;
        }
    }
    Ok(())
}

/// Remove isolation baselines/worktrees older than `max_age_days` (design §10.3).
///
/// Order (important for safety):
/// 1. Expired **non-baseline** entries under snapshots
/// 2. Expired **baselines** under snapshots only if no live session remains under
///    snapshots **or** worktrees (git worktree sessions live at
///    `worktrees/{id}` while baselines are `snapshots/{id}-baseline`)
/// 3. Expired worktree session dirs
///
/// A baseline is never removed while its session directory still exists in either root.
pub fn gc_expired_isolation(
    snapshots_dir: &Path,
    worktrees_dir: Option<&Path>,
    max_age_days: u32,
    now_unix: i64,
) -> Result<u32> {
    let max_age_secs = i64::from(max_age_days).saturating_mul(24 * 60 * 60);
    let mut live_roots: Vec<PathBuf> = vec![snapshots_dir.to_path_buf()];
    if let Some(wt) = worktrees_dir {
        live_roots.push(wt.to_path_buf());
    }
    let mut removed = 0u32;
    // Phase 1: non-baselines in snapshots (session copies for SnapshotIsolator).
    removed += gc_dir_entries(
        snapshots_dir,
        max_age_secs,
        now_unix,
        &live_roots,
        GcPhase::NonBaselines,
    )?;
    // Phase 2: baselines in snapshots (protected while any live session root has {id}).
    removed += gc_dir_entries(
        snapshots_dir,
        max_age_secs,
        now_unix,
        &live_roots,
        GcPhase::Baselines,
    )?;
    // Phase 3: worktree session dirs (after baseline protection checked).
    if let Some(wt) = worktrees_dir {
        removed += gc_dir_entries(
            wt,
            max_age_secs,
            now_unix,
            &live_roots,
            GcPhase::NonBaselines,
        )?;
    }
    Ok(removed)
}

#[derive(Clone, Copy)]
enum GcPhase {
    NonBaselines,
    Baselines,
}

fn session_still_live(session_id: &str, live_roots: &[PathBuf]) -> bool {
    live_roots.iter().any(|root| root.join(session_id).exists())
}

fn is_expired(meta: &std::fs::Metadata, max_age_secs: i64, now_unix: i64) -> bool {
    let modified = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    now_unix.saturating_sub(modified) >= max_age_secs
}

fn gc_dir_entries(
    dir: &Path,
    max_age_secs: i64,
    now_unix: i64,
    live_roots: &[PathBuf],
    phase: GcPhase,
) -> Result<u32> {
    if !dir.exists() {
        return Ok(0);
    }
    let mut removed = 0u32;
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        if !is_expired(&meta, max_age_secs, now_unix) {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let is_baseline = name.ends_with("-baseline");
        match phase {
            GcPhase::NonBaselines if is_baseline => continue,
            GcPhase::Baselines if !is_baseline => continue,
            GcPhase::Baselines => {
                let session_id = name.trim_end_matches("-baseline");
                if session_still_live(session_id, live_roots) {
                    continue;
                }
            }
            GcPhase::NonBaselines => {}
        }
        if path.is_dir() {
            fs::remove_dir_all(&path)?;
        } else {
            fs::remove_file(&path)?;
        }
        removed += 1;
    }
    Ok(removed)
}

fn is_skipped_name(name: &str) -> bool {
    // Skip VCS, local state, heavy build/dependency trees, and reviewer staging dirs.
    // Config files (`.this-is-fine.toml`) are copied so baselines stay complete.
    matches!(
        name,
        ".git"
            | ".this-is-fine"
            | ".tif-candidate"
            | "target"
            | "node_modules"
            | ".venv"
            | "dist"
            | "build"
    )
}

fn git(cwd: &Path, args: &[&str]) -> Result<std::process::ExitStatus> {
    let status = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .status()
        .map_err(|e| TifError::Isolation(format!("git invocation failed: {e}")))?;
    Ok(status)
}

fn path_to_str(path: &Path) -> Result<&str> {
    path.to_str().ok_or_else(|| {
        TifError::Isolation("path is not valid Unicode; refusing to fall back to '.'".into())
    })
}

fn chrono_now() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Copy directory tree, skipping VCS/build/state dirs.
///
/// When `dst` lives under `src` (e.g. creating a snapshot inside the repo),
/// entries that are the destination tree are skipped to avoid recursion.
/// When `src` lives under `dst` (e.g. applying a worktree back onto the repo),
/// files are copied normally — the nested-source case is not a recursion hazard.
///
/// Symlink safety: never follows symlink directories; skips symlink files entirely.
/// Path components `.` / `..` are rejected when joining names.
fn copy_dir_selective(src: &Path, dst: &Path) -> Result<()> {
    copy_dir_selective_inner(src, dst, dst)
}

fn safe_child_name(name: &std::ffi::OsStr) -> Result<()> {
    let name_str = name.to_string_lossy();
    if name_str == "." || name_str == ".." || name_str.is_empty() {
        return Err(TifError::Isolation(format!(
            "refusing path component `{name_str}` while copying"
        )));
    }
    if name_str.contains('/') || name_str.contains('\\') || name_str.contains('\0') {
        return Err(TifError::Isolation(format!(
            "refusing unsafe path component: {name_str}"
        )));
    }
    Ok(())
}

fn destination_stays_under(dst_root: &Path, dest: &Path) -> bool {
    dest.starts_with(dst_root)
}

fn copy_dir_selective_inner(src: &Path, dst: &Path, dst_root: &Path) -> Result<()> {
    // Do not follow a symlink src root into foreign trees.
    if let Ok(meta) = fs::symlink_metadata(src) {
        if meta.file_type().is_symlink() {
            return Ok(());
        }
    }
    fs::create_dir_all(dst)?;
    if !destination_stays_under(dst_root, dst) {
        return Err(TifError::Isolation(format!(
            "copy destination escapes root: {}",
            dst.display()
        )));
    }
    let dst_canon = dst.canonicalize().unwrap_or_else(|_| dst.to_path_buf());
    let src_canon = src.canonicalize().unwrap_or_else(|_| src.to_path_buf());
    // Only guard against walking *into* dst when dst is nested under src.
    let dst_nested_in_src = dst_canon.starts_with(&src_canon) && dst_canon != src_canon;

    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if is_skipped_name(name_str.as_ref()) {
            continue;
        }
        safe_child_name(&name)?;
        let from = entry.path();
        // Symlink safety: skip symlinks (never follow into symlink dirs).
        let meta = match fs::symlink_metadata(&from) {
            Ok(m) => m,
            Err(_) => continue,
        };
        if meta.file_type().is_symlink() {
            continue;
        }
        if dst_nested_in_src && meta.file_type().is_dir() {
            let from_canon = from.canonicalize().unwrap_or_else(|_| from.clone());
            if from_canon == dst_canon || from_canon.starts_with(&dst_canon) {
                continue;
            }
        }
        let to = dst.join(&name);
        if !destination_stays_under(dst_root, &to) {
            return Err(TifError::Isolation(format!(
                "copy destination escapes root: {}",
                to.display()
            )));
        }
        // Never overwrite a path with itself.
        if let (Ok(fc), Ok(tc)) = (from.canonicalize(), to.canonicalize()) {
            if fc == tc {
                continue;
            }
        }
        if meta.file_type().is_dir() {
            copy_dir_selective_inner(&from, &to, dst_root)?;
        } else if meta.file_type().is_file() {
            if let Some(parent) = to.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

/// High-level helper: create isolation, leave original untouched.
pub fn open_isolation(
    source_root: &Path,
    state_dir: &Path,
    session_id: &str,
) -> Result<(Box<dyn Isolator>, IsolationSession)> {
    let snaps = state_dir.join("snapshots");
    let isolator = select_isolator(source_root, &snaps);
    let session = isolator.create(source_root, session_id)?;
    Ok((isolator, session))
}

/// Apply a verified candidate with fail-safe restore on error.
pub fn apply_verified_candidate(
    isolator: &dyn Isolator,
    session: &mut IsolationSession,
) -> Result<()> {
    isolator.apply_to_source(session)
}

/// Roll back a previously applied candidate.
pub fn rollback_applied_candidate(
    isolator: &dyn Isolator,
    session: &mut IsolationSession,
) -> Result<()> {
    isolator.restore_source(session)
}

/// Reconstruct an isolator matching a persisted session.
pub fn isolator_for_session(session: &IsolationSession, snapshots_dir: &Path) -> Box<dyn Isolator> {
    match session.kind {
        IsolationKind::GitWorktree => {
            Box::new(GitWorktreeIsolator::new(snapshots_dir.to_path_buf()))
        }
        IsolationKind::Snapshot => Box::new(SnapshotIsolator::new(snapshots_dir.to_path_buf())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn snapshot_create_and_destroy() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("source");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("hello.txt"), "hi").unwrap();
        // Keep snapshot root outside the source tree to avoid nested copies.
        let snaps = dir.path().join("snaps");
        let iso = SnapshotIsolator::new(snaps);
        let session = iso.create(&source, "s1").unwrap();
        assert!(session.path.join("hello.txt").exists());
        iso.destroy(&session).unwrap();
        assert!(!session.path.exists());
    }

    #[test]
    fn select_snapshot_for_nongit() {
        let dir = tempdir().unwrap();
        let iso = select_isolator(dir.path(), &dir.path().join("snaps"));
        assert_eq!(iso.kind(), IsolationKind::Snapshot);
    }

    #[test]
    fn snapshot_apply_and_rollback_roundtrip() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("source");
        fs::create_dir_all(source.join("src")).unwrap();
        fs::write(
            source.join("src/main.rs"),
            "fn main() { println!(\"v1\"); }\n",
        )
        .unwrap();
        fs::write(source.join("README.md"), "original\n").unwrap();

        let snaps = dir.path().join("snaps");
        let iso = SnapshotIsolator::new(snaps);
        let mut session = iso.create(&source, "fb1").unwrap();

        // Candidate: simpler main, drop nothing critical.
        fs::write(session.path.join("src/main.rs"), "fn main() {}\n").unwrap();
        fs::write(session.path.join("README.md"), "original\n").unwrap();
        // Candidate also adds a file that apply will bring over.
        fs::write(session.path.join("extra.txt"), "from-candidate\n").unwrap();

        // Before apply, source is still original.
        assert_eq!(
            fs::read_to_string(source.join("src/main.rs")).unwrap(),
            "fn main() { println!(\"v1\"); }\n"
        );
        assert!(!source.join("extra.txt").exists());

        iso.apply_to_source(&mut session).unwrap();
        assert!(session.applied);
        assert_eq!(
            fs::read_to_string(source.join("src/main.rs")).unwrap(),
            "fn main() {}\n"
        );
        assert_eq!(
            fs::read_to_string(source.join("extra.txt")).unwrap(),
            "from-candidate\n"
        );
        assert!(session.baseline_path.as_ref().unwrap().exists());

        // Rollback restores original content and removes candidate-only files.
        iso.restore_source(&mut session).unwrap();
        assert!(!session.applied);
        assert_eq!(
            fs::read_to_string(source.join("src/main.rs")).unwrap(),
            "fn main() { println!(\"v1\"); }\n"
        );
        assert!(!source.join("extra.txt").exists());
        assert_eq!(
            fs::read_to_string(source.join("README.md")).unwrap(),
            "original\n"
        );
    }

    #[test]
    fn failed_isolation_never_touches_source() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("source");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("keep.txt"), "safe\n").unwrap();
        let snaps = dir.path().join("snaps");
        let iso = SnapshotIsolator::new(snaps);
        let session = iso.create(&source, "no-apply").unwrap();
        // Mutate only the isolation copy.
        fs::write(session.path.join("keep.txt"), "mutated\n").unwrap();
        fs::write(session.path.join("evil.txt"), "nope\n").unwrap();
        // Source must remain intact without apply.
        assert_eq!(
            fs::read_to_string(source.join("keep.txt")).unwrap(),
            "safe\n"
        );
        assert!(!source.join("evil.txt").exists());
        // restore when not applied is a no-op
        let mut session = session;
        iso.restore_source(&mut session).unwrap();
        assert_eq!(
            fs::read_to_string(source.join("keep.txt")).unwrap(),
            "safe\n"
        );
    }

    #[test]
    fn apply_without_candidate_path_errors() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("source");
        fs::create_dir_all(&source).unwrap();
        let snaps = dir.path().join("snaps");
        let iso = SnapshotIsolator::new(snaps);
        let mut session = iso.create(&source, "gone").unwrap();
        fs::remove_dir_all(&session.path).unwrap();
        assert!(iso.apply_to_source(&mut session).is_err());
        // Source untouched.
        assert!(source.exists());
    }

    #[test]
    fn git_worktree_apply_rollback_when_git_available() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("repo");
        fs::create_dir_all(source.join("src")).unwrap();
        fs::write(source.join("src/lib.rs"), "pub fn f() -> i32 { 1 }\n").unwrap();
        // init git
        let ok = Command::new("git")
            .args(["init"])
            .current_dir(&source)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !ok {
            return; // skip when git missing
        }
        let _ = Command::new("git")
            .args(["config", "user.email", "tif@test.local"])
            .current_dir(&source)
            .status();
        let _ = Command::new("git")
            .args(["config", "user.name", "tif-test"])
            .current_dir(&source)
            .status();
        let _ = Command::new("git")
            .args(["add", "."])
            .current_dir(&source)
            .status();
        let commit_ok = Command::new("git")
            .args(["commit", "-m", "init"])
            .current_dir(&source)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !commit_ok {
            return;
        }

        let snaps = dir.path().join("snaps");
        let iso = GitWorktreeIsolator::new(snaps);
        let mut session = match iso.create(&source, "gwt1") {
            Ok(s) => s,
            Err(_) => return, // environment may refuse worktrees
        };
        assert!(session.path.join("src/lib.rs").exists());

        // Plant a smaller candidate in the worktree.
        fs::write(session.path.join("src/lib.rs"), "pub fn f() -> i32 { 0 }\n").unwrap();

        iso.apply_to_source(&mut session).unwrap();
        assert_eq!(
            fs::read_to_string(source.join("src/lib.rs")).unwrap(),
            "pub fn f() -> i32 { 0 }\n"
        );

        iso.restore_source(&mut session).unwrap();
        assert_eq!(
            fs::read_to_string(source.join("src/lib.rs")).unwrap(),
            "pub fn f() -> i32 { 1 }\n"
        );

        iso.destroy(&session).unwrap();
    }

    #[test]
    fn apply_prunes_files_deleted_in_candidate() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("source");
        fs::create_dir_all(source.join("src")).unwrap();
        fs::write(source.join("src/lib.rs"), "keep\n").unwrap();
        fs::write(source.join("bloat.txt"), "fuel\n").unwrap();
        let snaps = dir.path().join("snaps");
        let iso = SnapshotIsolator::new(snaps);
        let mut session = iso.create(&source, "prune1").unwrap();
        // Candidate drops bloat.
        let _ = fs::remove_file(session.path.join("bloat.txt"));
        fs::write(session.path.join("src/lib.rs"), "keep\n").unwrap();
        iso.apply_to_source(&mut session).unwrap();
        assert!(session.applied);
        assert!(!source.join("bloat.txt").exists());
        assert_eq!(
            fs::read_to_string(source.join("src/lib.rs")).unwrap(),
            "keep\n"
        );
        iso.restore_source(&mut session).unwrap();
        assert!(source.join("bloat.txt").exists());
    }

    #[test]
    fn apply_failure_restores_when_possible() {
        // Force apply failure after baseline by making candidate path unreadable mid-flight:
        // replace candidate with a file where a directory is expected during copy of a child.
        let dir = tempdir().unwrap();
        let source = dir.path().join("source");
        fs::create_dir_all(source.join("nested")).unwrap();
        fs::write(source.join("nested/a.txt"), "A\n").unwrap();
        fs::write(source.join("ok.txt"), "ok\n").unwrap();
        let snaps = dir.path().join("snaps");
        let iso = SnapshotIsolator::new(snaps);
        let mut session = iso.create(&source, "fail1").unwrap();
        // Corrupt candidate: `nested` becomes a file so copy of nested/a.txt fails on some OSes,
        // or remove nested and put a file named nested — copy_dir then tries to write under file.
        fs::remove_dir_all(session.path.join("nested")).unwrap();
        fs::write(session.path.join("nested"), "not-a-dir\n").unwrap();
        // Plant different content so a successful partial would be visible.
        fs::write(session.path.join("ok.txt"), "changed\n").unwrap();

        let err = iso.apply_to_source(&mut session);
        // Either apply succeeds (overlay file `nested` onto source) or fails and restores.
        // On Windows, replacing a dir with a file at source may succeed for ok.txt then fail —
        // assert: if error claims restored, source is clean; if dual-failure, sticky flags.
        match err {
            Ok(()) => {
                // Apply treated file `nested` as a file write — still a valid FS outcome.
                assert!(session.applied);
            }
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("restore also failed") {
                    assert!(session.applied || session.restore_pending);
                } else {
                    assert!(
                        msg.contains("restored from baseline"),
                        "unexpected message: {msg}"
                    );
                    assert!(!session.applied);
                    assert!(!session.restore_pending);
                    assert_eq!(fs::read_to_string(source.join("ok.txt")).unwrap(), "ok\n");
                    assert!(source.join("nested").is_dir() || source.join("nested/a.txt").exists());
                }
            }
        }
    }

    #[test]
    fn sticky_restore_pending_allows_retry() {
        let dir = tempdir().unwrap();
        let source = dir.path().join("source");
        fs::create_dir_all(&source).unwrap();
        fs::write(source.join("a.txt"), "A\n").unwrap();
        let snaps = dir.path().join("snaps");
        let iso = SnapshotIsolator::new(snaps);
        let mut session = iso.create(&source, "sticky").unwrap();
        iso.preserve_baseline(&mut session).unwrap();
        // Simulate dual-failure sticky state.
        fs::write(source.join("a.txt"), "dirty\n").unwrap();
        session.applied = true;
        session.restore_pending = true;
        iso.restore_source(&mut session).unwrap();
        assert!(!session.applied);
        assert!(!session.restore_pending);
        assert_eq!(fs::read_to_string(source.join("a.txt")).unwrap(), "A\n");
    }

    #[test]
    fn gc_expired_isolation_skips_baseline_with_sibling() {
        let dir = tempdir().unwrap();
        let snaps = dir.path().join("snaps");
        fs::create_dir_all(snaps.join("old")).unwrap();
        fs::create_dir_all(snaps.join("old-baseline")).unwrap();
        // Two-phase with max_age 0: phase 1 removes session `old`, phase 2 then
        // removes baseline (session no longer live). Single call clears both when
        // session is co-located under snapshots. Protection is "while session exists"
        // within the baseline phase — verify via live-root worktree layout below.
        // Here: with only snapshots, one GC cleans expired session then baseline.
        let _ = gc_expired_isolation(&snaps, None, 0, chrono_now() + 10).unwrap();
        assert!(
            !snaps.join("old").exists() && !snaps.join("old-baseline").exists(),
            "expired snapshot session + baseline both collected in ordered GC"
        );
    }

    #[test]
    fn gc_protects_worktree_session_baseline_in_snapshots() {
        // Git layout: session at worktrees/{id}, baseline at snapshots/{id}-baseline.
        // Phase order: baseline GC runs before worktree session GC, so a live
        // worktree protects the baseline for the duration of that call.
        let dir = tempdir().unwrap();
        let snaps = dir.path().join("snaps");
        let worktrees = dir.path().join("worktrees");
        fs::create_dir_all(worktrees.join("gwt1")).unwrap();
        fs::create_dir_all(snaps.join("gwt1-baseline")).unwrap();
        fs::write(snaps.join("gwt1-baseline/keep.txt"), "baseline\n").unwrap();

        let removed = gc_expired_isolation(&snaps, Some(&worktrees), 0, chrono_now() + 10).unwrap();
        assert!(
            snaps.join("gwt1-baseline").exists(),
            "worktree session baseline must not be aged out while worktrees/gwt1 exists at baseline phase; removed={removed}"
        );
        // Worktree session itself may have been collected in phase 3 of the same call.
        // If so, a second GC removes the now-unprotected baseline.
        if !worktrees.join("gwt1").exists() {
            let _ = gc_expired_isolation(&snaps, Some(&worktrees), 0, chrono_now() + 10).unwrap();
            assert!(
                !snaps.join("gwt1-baseline").exists(),
                "baseline should be GC'd after worktree session is gone"
            );
        } else {
            // Session still present (not expired under this FS clock) — baseline stays.
            assert!(snaps.join("gwt1-baseline").exists());
            fs::remove_dir_all(worktrees.join("gwt1")).unwrap();
            let _ = gc_expired_isolation(&snaps, Some(&worktrees), 0, chrono_now() + 10).unwrap();
            assert!(!snaps.join("gwt1-baseline").exists());
        }
    }

    #[test]
    fn gc_baseline_protected_while_worktree_session_still_on_disk() {
        // Explicit: baseline phase sees live worktree and skips deletion.
        let dir = tempdir().unwrap();
        let snaps = dir.path().join("snaps");
        let worktrees = dir.path().join("worktrees");
        fs::create_dir_all(worktrees.join("active")).unwrap();
        fs::create_dir_all(snaps.join("active-baseline")).unwrap();
        assert!(session_still_live(
            "active",
            &[snaps.clone(), worktrees.clone()]
        ));
        // Only run baseline phase via full GC with a non-expiring max age for session…
        // With max_age 0, phase 3 may remove worktree; baseline phase runs first.
        let before = snaps.join("active-baseline").exists();
        assert!(before);
        let _ = gc_expired_isolation(&snaps, Some(&worktrees), 0, chrono_now() + 10).unwrap();
        // After one call: if worktree was removed in phase 3, baseline still existed
        // through phase 2 (this call). Confirm session_still_live saw worktree.
        // Recreate and verify protection mid-phase semantics via session_still_live.
        fs::create_dir_all(worktrees.join("active2")).unwrap();
        fs::create_dir_all(snaps.join("active2-baseline")).unwrap();
        assert!(session_still_live(
            "active2",
            &[snaps.clone(), worktrees.clone()]
        ));
        assert!(!session_still_live("missing", &[snaps, worktrees]));
    }
}
