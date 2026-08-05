//! Isolation primitives: Git worktree and non-Git snapshot/rollback interfaces.

use serde::{Deserialize, Serialize};
use std::fs;
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

/// Handle to an isolated workspace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IsolationSession {
    pub kind: IsolationKind,
    pub id: String,
    pub path: PathBuf,
    pub source_root: PathBuf,
    pub created_at_unix: i64,
}

/// Trait for isolation backends.
pub trait Isolator: Send + Sync {
    fn kind(&self) -> IsolationKind;
    fn create(&self, source_root: &Path, session_id: &str) -> Result<IsolationSession>;
    fn destroy(&self, session: &IsolationSession) -> Result<()>;
    /// Apply a verified candidate from isolation back to the source (or mark ready).
    fn apply_to_source(&self, session: &IsolationSession) -> Result<()>;
    /// Restore source to pre-Firebreak state when a candidate is discarded.
    fn restore_source(&self, session: &IsolationSession) -> Result<()>;
}

/// Select an isolator based on repository characteristics.
pub fn select_isolator(source_root: &Path, snapshots_dir: &Path) -> Box<dyn Isolator> {
    if source_root.join(".git").exists() {
        Box::new(GitWorktreeIsolator::new())
    } else {
        Box::new(SnapshotIsolator::new(snapshots_dir.to_path_buf()))
    }
}

/// Git worktree-based isolation.
#[derive(Debug, Default)]
pub struct GitWorktreeIsolator;

impl GitWorktreeIsolator {
    pub fn new() -> Self {
        Self
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

        let path_str = worktree_path.to_str().ok_or_else(|| {
            TifError::Isolation(
                "worktree path is not valid Unicode; refusing to fall back to '.'".into(),
            )
        })?;

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
            created_at_unix: chrono_now(),
        })
    }

    fn destroy(&self, session: &IsolationSession) -> Result<()> {
        let path_str = session.path.to_str().ok_or_else(|| {
            TifError::Isolation(
                "worktree path is not valid Unicode; refusing to fall back to '.'".into(),
            )
        })?;
        let _ = git(
            &session.source_root,
            &["worktree", "remove", "--force", path_str],
        );
        // Best-effort branch cleanup
        let branch = format!("tif/firebreak/{}", session.id);
        let _ = git(&session.source_root, &["branch", "-D", &branch]);
        if session.path.exists() {
            let _ = fs::remove_dir_all(&session.path);
        }
        Ok(())
    }

    fn apply_to_source(&self, session: &IsolationSession) -> Result<()> {
        // MVP interface: caller is expected to copy verified diffs.
        // We only validate the session path exists.
        if !session.path.exists() {
            return Err(TifError::Isolation("worktree path missing".into()));
        }
        Ok(())
    }

    fn restore_source(&self, session: &IsolationSession) -> Result<()> {
        // Original workspace was never modified by unverified Firebreak.
        let _ = session;
        Ok(())
    }
}

/// Non-Git snapshot and rollback via directory copy.
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
        copy_dir_selective(source_root, &snap)?;
        Ok(IsolationSession {
            kind: IsolationKind::Snapshot,
            id: session_id.to_string(),
            path: snap,
            source_root: source_root.to_path_buf(),
            created_at_unix: chrono_now(),
        })
    }

    fn destroy(&self, session: &IsolationSession) -> Result<()> {
        if session.path.exists() {
            fs::remove_dir_all(&session.path)?;
        }
        Ok(())
    }

    fn apply_to_source(&self, session: &IsolationSession) -> Result<()> {
        // Apply means: snapshot became the candidate workspace; verified files would be merged.
        // Interface only for MVP — full patch application is a later pass.
        if !session.path.exists() {
            return Err(TifError::Isolation("snapshot path missing".into()));
        }
        Ok(())
    }

    fn restore_source(&self, session: &IsolationSession) -> Result<()> {
        // Restore from a baseline snapshot if we had snapshotted source first.
        let baseline = self.snapshots_dir.join(format!("{}-baseline", session.id));
        if baseline.exists() {
            // Remove source contents carefully is dangerous; MVP documents the interface.
            let _ = baseline;
        }
        Ok(())
    }
}

fn git(cwd: &Path, args: &[&str]) -> Result<std::process::ExitStatus> {
    let status = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .status()
        .map_err(|e| TifError::Isolation(format!("git invocation failed: {e}")))?;
    Ok(status)
}

fn chrono_now() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Copy directory tree, skipping VCS/build dirs and never walking into `dst`.
fn copy_dir_selective(src: &Path, dst: &Path) -> Result<()> {
    fs::create_dir_all(dst)?;
    let dst_canon = dst.canonicalize().unwrap_or_else(|_| dst.to_path_buf());
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if matches!(
            name_str.as_ref(),
            ".git" | ".this-is-fine" | "target" | "node_modules"
        ) {
            continue;
        }
        let from = entry.path();
        // Avoid infinite recursion when the snapshot directory lives under src.
        if from == dst || from.starts_with(&dst_canon) || dst.starts_with(&from) {
            continue;
        }
        let to = dst.join(&name);
        if from.is_dir() {
            copy_dir_selective(&from, &to)?;
        } else {
            fs::copy(&from, &to)?;
        }
    }
    Ok(())
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
}
