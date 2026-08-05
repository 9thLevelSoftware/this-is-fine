//! Repository inspector: roots, languages, verification discovery.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

use crate::error::Result;
use crate::verify::VerificationCategory;

/// Result of repository inspection.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RepoInspection {
    pub root: PathBuf,
    pub is_git: bool,
    pub languages: Vec<String>,
    pub package_managers: Vec<String>,
    pub frameworks: Vec<String>,
    pub verification_commands: Vec<DiscoveredCommand>,
    pub sensitive_hints: Vec<String>,
    pub unresolved_notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DiscoveredCommand {
    pub command: String,
    pub category: VerificationCategory,
    pub evidence: String,
    pub confident: bool,
}

/// Inspect a repository without inventing low-confidence commands.
#[derive(Debug, Default)]
pub struct RepositoryInspector;

impl RepositoryInspector {
    pub fn new() -> Self {
        Self
    }

    pub fn inspect(&self, root: &Path) -> Result<RepoInspection> {
        let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
        let is_git = root.join(".git").exists();

        let mut languages = Vec::new();
        let mut package_managers = Vec::new();
        let frameworks: Vec<String> = Vec::new();
        let mut verification_commands = Vec::new();
        let mut unresolved_notes = Vec::new();
        let mut sensitive_hints = Vec::new();

        // Rust / Cargo
        if root.join("Cargo.toml").exists() {
            languages.push("rust".into());
            package_managers.push("cargo".into());
            verification_commands.push(DiscoveredCommand {
                command: "cargo test".into(),
                category: VerificationCategory::UnitTest,
                evidence: "Cargo.toml present".into(),
                confident: true,
            });
            // rustfmt is conventional but not universal — only require when evidence exists.
            let has_rustfmt =
                root.join("rustfmt.toml").exists() || root.join(".rustfmt.toml").exists();
            verification_commands.push(DiscoveredCommand {
                command: "cargo fmt --check".into(),
                category: VerificationCategory::Format,
                evidence: if has_rustfmt {
                    "rustfmt.toml present".into()
                } else {
                    "Cargo.toml present (fmt suggested, not required without rustfmt.toml)".into()
                },
                confident: has_rustfmt,
            });
        }

        // Node
        if root.join("package.json").exists() {
            languages.push("javascript".into());
            package_managers.push("npm".into());
            if root.join("pnpm-lock.yaml").exists() {
                package_managers.push("pnpm".into());
            }
            if root.join("yarn.lock").exists() {
                package_managers.push("yarn".into());
            }
            // Do not invent test commands from package.json without parsing scripts confidently.
            if let Ok(text) = fs::read_to_string(root.join("package.json")) {
                if text.contains("\"test\"") {
                    verification_commands.push(DiscoveredCommand {
                        command: "npm test".into(),
                        category: VerificationCategory::UnitTest,
                        evidence: "package.json contains a test script".into(),
                        confident: true,
                    });
                } else {
                    unresolved_notes.push(
                        "package.json present but no test script detected; not inventing a command"
                            .into(),
                    );
                }
            }
        }

        // Python
        if root.join("pyproject.toml").exists() || root.join("requirements.txt").exists() {
            languages.push("python".into());
            if root.join("pyproject.toml").exists() {
                package_managers.push("pip/pyproject".into());
            }
            if root.join("pytest.ini").exists()
                || root.join("pyproject.toml").exists()
                    && fs::read_to_string(root.join("pyproject.toml"))
                        .map(|t| t.contains("pytest"))
                        .unwrap_or(false)
            {
                verification_commands.push(DiscoveredCommand {
                    command: "pytest".into(),
                    category: VerificationCategory::UnitTest,
                    evidence: "pytest configuration detected".into(),
                    confident: true,
                });
            } else {
                unresolved_notes.push(
                    "Python project detected but no confident test runner; left unresolved".into(),
                );
            }
        }

        // Go
        if root.join("go.mod").exists() {
            languages.push("go".into());
            package_managers.push("go modules".into());
            verification_commands.push(DiscoveredCommand {
                command: "go test ./...".into(),
                category: VerificationCategory::UnitTest,
                evidence: "go.mod present".into(),
                confident: true,
            });
        }

        // Sensitive path hints
        for hint in [
            "src/auth",
            "migrations",
            ".github/workflows",
            "deploy",
            "infra",
        ] {
            if path_exists_under(&root, hint) {
                sensitive_hints.push(hint.into());
            }
        }

        if languages.is_empty() {
            unresolved_notes.push("no well-known project markers found".into());
        }

        Ok(RepoInspection {
            root,
            is_git,
            languages,
            package_managers,
            frameworks,
            verification_commands,
            sensitive_hints,
            unresolved_notes,
        })
    }
}

fn path_exists_under(root: &Path, rel: &str) -> bool {
    let p = root.join(rel);
    p.exists()
}

/// Find repository root by walking up for `.this-is-fine.toml` or `.git`.
pub fn find_repo_root(start: &Path) -> PathBuf {
    let mut current = start.canonicalize().unwrap_or_else(|_| start.to_path_buf());
    loop {
        if current.join(crate::config::SHARED_CONFIG_NAME).exists() || current.join(".git").exists()
        {
            return current;
        }
        if !current.pop() {
            break;
        }
    }
    start.canonicalize().unwrap_or_else(|_| start.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn detects_cargo() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("Cargo.toml"), "[package]\nname=\"x\"\n").unwrap();
        let insp = RepositoryInspector::new().inspect(dir.path()).unwrap();
        assert!(insp.languages.contains(&"rust".into()));
        assert!(insp
            .verification_commands
            .iter()
            .any(|c| c.command.contains("cargo test")));
    }

    #[test]
    fn does_not_invent_python_tests() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("requirements.txt"), "requests\n").unwrap();
        let insp = RepositoryInspector::new().inspect(dir.path()).unwrap();
        assert!(insp.languages.contains(&"python".into()));
        assert!(insp.verification_commands.is_empty());
        assert!(!insp.unresolved_notes.is_empty());
    }
}
