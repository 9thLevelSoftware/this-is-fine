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
        let mut frameworks: Vec<String> = Vec::new();
        let mut verification_commands = Vec::new();
        let mut unresolved_notes = Vec::new();
        let mut sensitive_hints = Vec::new();

        inspect_rust(
            &root,
            &mut languages,
            &mut package_managers,
            &mut verification_commands,
        );
        inspect_node(
            &root,
            &mut languages,
            &mut package_managers,
            &mut frameworks,
            &mut verification_commands,
            &mut unresolved_notes,
        );
        inspect_python(
            &root,
            &mut languages,
            &mut package_managers,
            &mut verification_commands,
            &mut unresolved_notes,
        );
        inspect_go(
            &root,
            &mut languages,
            &mut package_managers,
            &mut verification_commands,
        );

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

fn inspect_rust(
    root: &Path,
    languages: &mut Vec<String>,
    package_managers: &mut Vec<String>,
    verification_commands: &mut Vec<DiscoveredCommand>,
) {
    if !root.join("Cargo.toml").exists() {
        return;
    }
    languages.push("rust".into());
    package_managers.push("cargo".into());
    verification_commands.push(DiscoveredCommand {
        command: "cargo test".into(),
        category: VerificationCategory::UnitTest,
        evidence: "Cargo.toml present".into(),
        confident: true,
    });
    // rustfmt is conventional but not universal — only require when evidence exists.
    let has_rustfmt = root.join("rustfmt.toml").exists() || root.join(".rustfmt.toml").exists();
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
    let has_clippy = root.join("clippy.toml").exists() || root.join(".clippy.toml").exists();
    verification_commands.push(DiscoveredCommand {
        command: "cargo clippy --all-targets -- -D warnings".into(),
        category: VerificationCategory::Lint,
        evidence: if has_clippy {
            "clippy.toml present".into()
        } else {
            "Cargo.toml present (clippy suggested, not required without clippy.toml)".into()
        },
        confident: has_clippy,
    });
}

fn inspect_node(
    root: &Path,
    languages: &mut Vec<String>,
    package_managers: &mut Vec<String>,
    frameworks: &mut Vec<String>,
    verification_commands: &mut Vec<DiscoveredCommand>,
    unresolved_notes: &mut Vec<String>,
) {
    let pkg_path = root.join("package.json");
    if !pkg_path.exists() {
        return;
    }

    let text = match fs::read_to_string(&pkg_path) {
        Ok(t) => t,
        Err(_) => {
            languages.push("javascript".into());
            package_managers.push("npm".into());
            unresolved_notes.push("package.json unreadable; not inventing commands".into());
            return;
        }
    };

    // Language: TypeScript when tsconfig or typescript dependency present.
    let has_tsconfig = root.join("tsconfig.json").exists()
        || root.join("tsconfig.base.json").exists()
        || root.join("tsconfig.build.json").exists();
    let mentions_typescript = text.contains("\"typescript\"")
        || text.contains("\"@types/")
        || text.contains(".ts")
        || has_tsconfig;
    if mentions_typescript || has_tsconfig {
        languages.push("typescript".into());
    }
    languages.push("javascript".into());

    // Package manager preference: lockfiles first.
    let mut primary_pm = "npm";
    if root.join("pnpm-lock.yaml").exists() {
        package_managers.push("pnpm".into());
        primary_pm = "pnpm";
    }
    if root.join("yarn.lock").exists() {
        package_managers.push("yarn".into());
        if primary_pm == "npm" {
            primary_pm = "yarn";
        }
    }
    if root.join("bun.lockb").exists() || root.join("bun.lock").exists() {
        package_managers.push("bun".into());
        if primary_pm == "npm" {
            primary_pm = "bun";
        }
    }
    if !package_managers.iter().any(|m| m == "npm") {
        // Always note npm as the package.json baseline when no other lockfile won.
        package_managers.insert(0, "npm".into());
    } else if primary_pm == "npm" {
        package_managers.push("npm".into());
    }
    // Ensure primary is first for command selection.
    if let Some(pos) = package_managers.iter().position(|m| m == primary_pm) {
        if pos != 0 {
            let m = package_managers.remove(pos);
            package_managers.insert(0, m);
        }
    }

    // Framework hints (non-command; informational).
    for (needle, name) in [
        ("\"next\"", "next"),
        ("\"react\"", "react"),
        ("\"vue\"", "vue"),
        ("\"@angular/core\"", "angular"),
        ("\"vitest\"", "vitest"),
        ("\"jest\"", "jest"),
        ("\"mocha\"", "mocha"),
    ] {
        if text.contains(needle) && !frameworks.iter().any(|f| f == name) {
            frameworks.push(name.into());
        }
    }

    let scripts = parse_package_json_scripts(&text);
    let run_prefix = match primary_pm {
        "pnpm" => "pnpm",
        "yarn" => "yarn",
        "bun" => "bun",
        _ => "npm",
    };

    // Prefer well-known script names; only emit when script key is present.
    let script_map: &[(&str, VerificationCategory, bool)] = &[
        ("test", VerificationCategory::UnitTest, true),
        ("test:unit", VerificationCategory::UnitTest, true),
        ("lint", VerificationCategory::Lint, true),
        ("typecheck", VerificationCategory::TypeCheck, true),
        ("type-check", VerificationCategory::TypeCheck, true),
        ("format", VerificationCategory::Format, false),
        ("fmt", VerificationCategory::Format, false),
        ("build", VerificationCategory::Build, false),
    ];

    let mut found_test = false;
    for (script, category, confident) in script_map {
        if !scripts.iter().any(|s| s == script) {
            continue;
        }
        if *category == VerificationCategory::UnitTest {
            found_test = true;
        }
        let command = match run_prefix {
            "npm" => format!("npm run {script}"),
            "pnpm" => format!("pnpm run {script}"),
            "yarn" => {
                // yarn run <script> works for all; yarn test is sugar for "test".
                if *script == "test" {
                    "yarn test".into()
                } else {
                    format!("yarn run {script}")
                }
            }
            "bun" => format!("bun run {script}"),
            _ => format!("{run_prefix} run {script}"),
        };
        // npm test is the conventional form for the "test" script.
        let command = if run_prefix == "npm" && *script == "test" {
            "npm test".into()
        } else {
            command
        };
        verification_commands.push(DiscoveredCommand {
            command,
            category: *category,
            evidence: format!("package.json scripts.{script} present ({run_prefix})"),
            confident: *confident,
        });
    }

    // tsc --noEmit only when tsconfig exists AND no typecheck script already found.
    if has_tsconfig
        && !verification_commands
            .iter()
            .any(|c| c.category == VerificationCategory::TypeCheck)
    {
        verification_commands.push(DiscoveredCommand {
            command: "npx tsc --noEmit".into(),
            category: VerificationCategory::TypeCheck,
            evidence: "tsconfig.json present without typecheck script (suggested)".into(),
            confident: false,
        });
    }

    if !found_test {
        unresolved_notes.push(
            "package.json present but no test/test:unit script detected; not inventing a command"
                .into(),
        );
    }
}

/// Extract top-level `"scripts"` object keys from package.json text (lightweight parse).
fn parse_package_json_scripts(text: &str) -> Vec<String> {
    // Prefer serde_json when the file is valid JSON.
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(text) {
        if let Some(scripts) = v.get("scripts").and_then(|s| s.as_object()) {
            return scripts.keys().cloned().collect();
        }
        return Vec::new();
    }
    // Fallback: scan for "scripts" section keys heuristically.
    let mut keys = Vec::new();
    if let Some(idx) = text.find("\"scripts\"") {
        let after = &text[idx..];
        if let Some(brace) = after.find('{') {
            let section = &after[brace..];
            let end = section.find('}').unwrap_or(section.len());
            let body = &section[1..end];
            for part in body.split(',') {
                if let Some(q1) = part.find('"') {
                    let rest = &part[q1 + 1..];
                    if let Some(q2) = rest.find('"') {
                        let key = &rest[..q2];
                        if !key.is_empty() && !key.contains('\n') {
                            keys.push(key.to_string());
                        }
                    }
                }
            }
        }
    }
    keys
}

fn inspect_python(
    root: &Path,
    languages: &mut Vec<String>,
    package_managers: &mut Vec<String>,
    verification_commands: &mut Vec<DiscoveredCommand>,
    unresolved_notes: &mut Vec<String>,
) {
    let has_pyproject = root.join("pyproject.toml").exists();
    let has_requirements = root.join("requirements.txt").exists();
    let has_setup = root.join("setup.py").exists() || root.join("setup.cfg").exists();
    if !has_pyproject && !has_requirements && !has_setup {
        return;
    }

    languages.push("python".into());
    if has_pyproject {
        package_managers.push("pip/pyproject".into());
    } else if has_requirements {
        package_managers.push("pip".into());
    }

    let pyproject = if has_pyproject {
        fs::read_to_string(root.join("pyproject.toml")).unwrap_or_default()
    } else {
        String::new()
    };

    let has_pytest_ini = root.join("pytest.ini").exists();
    let has_tox = root.join("tox.ini").exists();
    let has_conftest = root.join("conftest.py").exists()
        || root.join("tests").join("conftest.py").exists()
        || root.join("test").join("conftest.py").exists();
    let pyproject_pytest = pyproject.contains("pytest")
        || pyproject.contains("[tool.pytest")
        || pyproject.contains("pytest.ini_options");
    let pyproject_has_test_script = pyproject.contains("[tool.poetry.scripts]")
        && pyproject.to_ascii_lowercase().contains("pytest");

    // Poetry / hatch / pdm test runners only when explicitly configured.
    if pyproject.contains("[tool.poetry") && package_managers.iter().all(|m| m != "poetry") {
        package_managers.push("poetry".into());
    }
    if pyproject.contains("[tool.pdm") && package_managers.iter().all(|m| m != "pdm") {
        package_managers.push("pdm".into());
    }
    if pyproject.contains("[tool.hatch") && package_managers.iter().all(|m| m != "hatch") {
        package_managers.push("hatch".into());
    }

    let confident_pytest =
        has_pytest_ini || pyproject_pytest || has_conftest || pyproject_has_test_script;
    if confident_pytest {
        verification_commands.push(DiscoveredCommand {
            command: "pytest".into(),
            category: VerificationCategory::UnitTest,
            evidence: if has_pytest_ini {
                "pytest.ini present".into()
            } else if pyproject_pytest {
                "pyproject.toml pytest configuration detected".into()
            } else if has_conftest {
                "conftest.py present".into()
            } else {
                "pytest referenced in project config".into()
            },
            confident: true,
        });
    } else if has_tox {
        verification_commands.push(DiscoveredCommand {
            command: "tox".into(),
            category: VerificationCategory::UnitTest,
            evidence: "tox.ini present".into(),
            confident: true,
        });
    } else if has_pyproject && pyproject.contains("[tool.poetry") {
        // Poetry projects often use `poetry run pytest` but without evidence we stay weak.
        verification_commands.push(DiscoveredCommand {
            command: "poetry run pytest".into(),
            category: VerificationCategory::UnitTest,
            evidence: "poetry project without pytest config (suggested, not required)".into(),
            confident: false,
        });
        unresolved_notes.push(
            "Python poetry project detected but no confident test runner; suggested weak command"
                .into(),
        );
    } else {
        unresolved_notes
            .push("Python project detected but no confident test runner; left unresolved".into());
    }

    // Ruff / black / mypy only with config evidence.
    if pyproject.contains("[tool.ruff") || root.join("ruff.toml").exists() {
        verification_commands.push(DiscoveredCommand {
            command: "ruff check .".into(),
            category: VerificationCategory::Lint,
            evidence: "ruff configuration detected".into(),
            confident: true,
        });
    }
    if pyproject.contains("[tool.mypy") || root.join("mypy.ini").exists() {
        verification_commands.push(DiscoveredCommand {
            command: "mypy .".into(),
            category: VerificationCategory::TypeCheck,
            evidence: "mypy configuration detected".into(),
            confident: true,
        });
    }
}

fn inspect_go(
    root: &Path,
    languages: &mut Vec<String>,
    package_managers: &mut Vec<String>,
    verification_commands: &mut Vec<DiscoveredCommand>,
) {
    if !root.join("go.mod").exists() {
        return;
    }
    languages.push("go".into());
    package_managers.push("go modules".into());
    verification_commands.push(DiscoveredCommand {
        command: "go test ./...".into(),
        category: VerificationCategory::UnitTest,
        evidence: "go.mod present".into(),
        confident: true,
    });
    // gofmt / go vet are conventional but not always desired as hard gates.
    verification_commands.push(DiscoveredCommand {
        command: "go vet ./...".into(),
        category: VerificationCategory::Lint,
        evidence: "go.mod present (vet suggested)".into(),
        confident: false,
    });
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
            .any(|c| c.command.contains("cargo test") && c.confident));
        // clippy without clippy.toml is not confident
        let clippy = insp
            .verification_commands
            .iter()
            .find(|c| c.command.contains("clippy"))
            .unwrap();
        assert!(!clippy.confident);
    }

    #[test]
    fn does_not_invent_python_tests() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("requirements.txt"), "requests\n").unwrap();
        let insp = RepositoryInspector::new().inspect(dir.path()).unwrap();
        assert!(insp.languages.contains(&"python".into()));
        assert!(insp
            .verification_commands
            .iter()
            .all(|c| !c.confident || c.category != VerificationCategory::UnitTest));
        assert!(insp.verification_commands.is_empty() || !insp.unresolved_notes.is_empty());
        assert!(!insp.unresolved_notes.is_empty());
    }

    #[test]
    fn package_json_scripts_npm_test_confident() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("package.json"),
            r#"{"name":"x","scripts":{"test":"jest","lint":"eslint ."}}"#,
        )
        .unwrap();
        let insp = RepositoryInspector::new().inspect(dir.path()).unwrap();
        assert!(insp.languages.contains(&"javascript".into()));
        let test_cmd = insp
            .verification_commands
            .iter()
            .find(|c| c.category == VerificationCategory::UnitTest)
            .expect("test command");
        assert!(test_cmd.confident);
        assert!(test_cmd.command.contains("test"));
        assert!(insp
            .verification_commands
            .iter()
            .any(|c| c.command.contains("lint")));
    }

    #[test]
    fn package_json_without_test_does_not_invent() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("package.json"),
            r#"{"name":"x","scripts":{"build":"tsc"}}"#,
        )
        .unwrap();
        let insp = RepositoryInspector::new().inspect(dir.path()).unwrap();
        assert!(!insp
            .verification_commands
            .iter()
            .any(|c| c.category == VerificationCategory::UnitTest && c.confident));
        assert!(insp
            .unresolved_notes
            .iter()
            .any(|n| n.contains("no test") || n.contains("not inventing")));
    }

    #[test]
    fn pnpm_lock_selects_pnpm_runner() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("package.json"),
            r#"{"name":"x","scripts":{"test":"vitest"}}"#,
        )
        .unwrap();
        fs::write(dir.path().join("pnpm-lock.yaml"), "lockfileVersion: '9'\n").unwrap();
        let insp = RepositoryInspector::new().inspect(dir.path()).unwrap();
        assert!(insp.package_managers.iter().any(|m| m == "pnpm"));
        assert!(insp
            .verification_commands
            .iter()
            .any(|c| c.command.starts_with("pnpm") && c.command.contains("test")));
    }

    #[test]
    fn typescript_tsconfig_weak_typecheck() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("package.json"),
            r#"{"name":"x","scripts":{"test":"node test.js"}}"#,
        )
        .unwrap();
        fs::write(
            dir.path().join("tsconfig.json"),
            r#"{"compilerOptions":{}}"#,
        )
        .unwrap();
        let insp = RepositoryInspector::new().inspect(dir.path()).unwrap();
        assert!(insp.languages.contains(&"typescript".into()));
        let tc = insp
            .verification_commands
            .iter()
            .find(|c| c.category == VerificationCategory::TypeCheck)
            .expect("typecheck");
        assert!(!tc.confident);
        assert!(tc.command.contains("tsc"));
    }

    #[test]
    fn python_pytest_ini_confident() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("pyproject.toml"), "[project]\nname=\"x\"\n").unwrap();
        fs::write(dir.path().join("pytest.ini"), "[pytest]\n").unwrap();
        let insp = RepositoryInspector::new().inspect(dir.path()).unwrap();
        let t = insp
            .verification_commands
            .iter()
            .find(|c| c.command == "pytest")
            .expect("pytest");
        assert!(t.confident);
    }

    #[test]
    fn python_pyproject_tool_pytest() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("pyproject.toml"),
            "[project]\nname=\"x\"\n[tool.pytest.ini_options]\nminversion = \"6.0\"\n",
        )
        .unwrap();
        let insp = RepositoryInspector::new().inspect(dir.path()).unwrap();
        assert!(insp
            .verification_commands
            .iter()
            .any(|c| c.command == "pytest" && c.confident));
    }

    #[test]
    fn go_mod_discovers_test() {
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("go.mod"),
            "module example.com/x\n\ngo 1.22\n",
        )
        .unwrap();
        let insp = RepositoryInspector::new().inspect(dir.path()).unwrap();
        assert!(insp.languages.contains(&"go".into()));
        let t = insp
            .verification_commands
            .iter()
            .find(|c| c.command.contains("go test"))
            .expect("go test");
        assert!(t.confident);
        // go vet is weak evidence
        assert!(insp
            .verification_commands
            .iter()
            .any(|c| c.command.contains("go vet") && !c.confident));
    }

    #[test]
    fn multi_lang_workspace_surfaces_all() {
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("Cargo.toml"), "[package]\nname=\"x\"\n").unwrap();
        fs::write(
            dir.path().join("package.json"),
            r#"{"name":"x","scripts":{"test":"jest"}}"#,
        )
        .unwrap();
        fs::write(dir.path().join("go.mod"), "module example.com/x\ngo 1.22\n").unwrap();
        fs::write(
            dir.path().join("pyproject.toml"),
            "[tool.pytest.ini_options]\n",
        )
        .unwrap();
        let insp = RepositoryInspector::new().inspect(dir.path()).unwrap();
        assert!(insp.languages.contains(&"rust".into()));
        assert!(insp.languages.contains(&"javascript".into()));
        assert!(insp.languages.contains(&"go".into()));
        assert!(insp.languages.contains(&"python".into()));
        // No invented commands beyond evidence-backed ones
        for c in &insp.verification_commands {
            if c.confident {
                assert!(
                    !c.evidence.contains("invent"),
                    "confident command must have real evidence: {:?}",
                    c
                );
            }
        }
    }
}
