//! Residual readiness rails: P1-8 upgrade/uninstall + F5 adapter install smoke.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tif_e2e::{assert_scenario, ensure_tif_built, run_scenario, tif_bin, workspace_root};

/// P1-8: real install scripts → reinstall (upgrade) → uninstall removes binary.
#[test]
fn p18_upgrade_uninstall_from_source() {
    ensure_tif_built();
    let r = run_scenario("P18", || {
        let prefix = tempfile::tempdir().map_err(|e| e.to_string())?;
        let secrets_home = tempfile::tempdir().map_err(|e| e.to_string())?;

        let bin = install_via_script(prefix.path())?;
        if !bin.is_file() {
            return Err(format!("missing binary after install: {}", bin.display()));
        }
        let v1 = run_bin_version(&bin)?;

        // Upgrade: install scripts again into the same prefix.
        let bin2 = install_via_script(prefix.path())?;
        if !bin2.is_file() {
            return Err("missing binary after upgrade install".into());
        }
        let v2 = run_bin_version(&bin2)?;
        if v1.is_empty() || v2.is_empty() {
            return Err(format!("empty version: v1={v1:?} v2={v2:?}"));
        }

        // Plant secrets matching credentials::secrets_dir() layout:
        // Unix: $XDG_CONFIG_HOME/tif/secrets or ~/.config/tif/secrets
        // Windows: %APPDATA%/tif/secrets
        let secrets = if cfg!(windows) {
            secrets_home
                .path()
                .join("AppData")
                .join("Roaming")
                .join("tif")
                .join("secrets")
        } else {
            secrets_home
                .path()
                .join(".config")
                .join("tif")
                .join("secrets")
        };
        fs::create_dir_all(&secrets).map_err(|e| e.to_string())?;
        fs::write(secrets.join("key"), b"secret-material").map_err(|e| e.to_string())?;
        // Also plant under PREFIX/secrets for custom-prefix purge.
        let pref_secrets = prefix.path().join("secrets");
        fs::create_dir_all(&pref_secrets).map_err(|e| e.to_string())?;
        fs::write(pref_secrets.join("key2"), b"more-secret").map_err(|e| e.to_string())?;

        uninstall_prefix(prefix.path(), secrets_home.path(), true)?;
        if bin.is_file() || bin2.is_file() {
            return Err("binary still present after uninstall".into());
        }
        if secrets.exists() {
            return Err(format!(
                "HOME secrets still present after --purge-secrets: {}",
                secrets.display()
            ));
        }
        if pref_secrets.exists() {
            return Err(format!(
                "PREFIX/secrets still present after purge: {}",
                pref_secrets.display()
            ));
        }

        Ok(format!(
            "real install+upgrade+uninstall+purge ok versions={v1}/{v2} prefix={}",
            prefix.path().display()
        ))
    });
    assert_scenario(&r);
}

/// F5 preparatory: adapter install scripts copy artifacts; protocol lifecycle still B12.
/// Does **not** claim F5 Pass without a real agent host product.
#[test]
fn f5_adapter_install_scripts_smoke() {
    ensure_tif_built();
    let r = run_scenario("F5-smoke", || {
        let root = workspace_root();
        let adapters = ["claude-code", "codex", "gemini-cli", "opencode"];
        let mut notes = Vec::new();

        for name in adapters {
            let sh = root.join("adapters").join(name).join("install.sh");
            let ps = root.join("adapters").join(name).join("install.ps1");
            if !sh.is_file() && !ps.is_file() {
                return Err(format!("{name}: missing install.sh and install.ps1"));
            }

            let home = tempfile::tempdir().map_err(|e| e.to_string())?;
            let cwd = tempfile::tempdir().map_err(|e| e.to_string())?;
            fs::write(cwd.path().join("README.md"), b"adapter smoke").map_err(|e| e.to_string())?;

            if sh.is_file() {
                if let Some(bash) = find_bash() {
                    // Per-adapter CLI shapes (documented in each install.md):
                    // claude-code: skills dir; codex: AGENTS.md path; others: optional dest.
                    let arg = match name {
                        "claude-code" => home.path().join("skills/this-is-fine"),
                        "codex" => cwd.path().join("AGENTS.md"),
                        _ => home.path().join(name),
                    };
                    if name == "codex" {
                        fs::write(&arg, b"# agents\n").map_err(|e| e.to_string())?;
                    }
                    let status = Command::new(&bash)
                        .arg(to_bash_path(&sh))
                        .arg(to_bash_path(&arg))
                        .current_dir(cwd.path())
                        .env("HOME", home.path())
                        .status()
                        .map_err(|e| format!("{name} install.sh spawn: {e}"))?;
                    if !status.success() {
                        return Err(format!("{name} install.sh failed: {status:?}"));
                    }
                    notes.push(format!("{name}:sh-ok"));
                    continue;
                }
            }

            if ps.is_file() && cfg!(windows) {
                let mut cmd = Command::new("powershell");
                cmd.args([
                    "-NoProfile",
                    "-ExecutionPolicy",
                    "Bypass",
                    "-File",
                    &ps.display().to_string(),
                ])
                .current_dir(cwd.path());
                if name == "claude-code" {
                    let skills = home.path().join("skills").join("this-is-fine");
                    cmd.arg("-SkillsDir").arg(skills.display().to_string());
                }
                let status = cmd
                    .status()
                    .map_err(|e| format!("{name} install.ps1 spawn: {e}"))?;
                if !status.success() {
                    return Err(format!("{name} install.ps1 failed: {status:?}"));
                }
                notes.push(format!("{name}:ps-ok"));
            } else {
                notes.push(format!("{name}:files-present"));
            }
        }

        let out = Command::new(tif_bin())
            .arg("--version")
            .output()
            .map_err(|e| e.to_string())?;
        if !out.status.success() {
            return Err("tif --version failed".into());
        }

        Ok(format!(
            "adapter install smoke: {}; tif ok (F5 vendor host still required for Must Pass)",
            notes.join(", ")
        ))
    });
    assert_scenario(&r);
}

/// Invoke real install scripts (`--from-source` / `-FromSource`).
fn install_via_script(prefix: &Path) -> Result<PathBuf, String> {
    let root = workspace_root();
    let status = if cfg!(windows) {
        Command::new("powershell")
            .args([
                "-NoProfile",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
                &root.join("scripts/install.ps1").display().to_string(),
                "-FromSource",
                "-Prefix",
                &prefix.display().to_string(),
            ])
            .current_dir(&root)
            .status()
            .map_err(|e| e.to_string())?
    } else {
        let bash = find_bash().ok_or("bash required for install.sh")?;
        let script = to_bash_path(&root.join("scripts/install.sh"));
        let pref = to_bash_path(prefix);
        Command::new(bash)
            .args([script.as_str(), "--from-source", "--prefix", pref.as_str()])
            .current_dir(&root)
            .status()
            .map_err(|e| e.to_string())?
    };
    if !status.success() {
        return Err(format!("install script failed: {status:?}"));
    }
    let bin = if cfg!(windows) {
        prefix.join("bin").join("tif.exe")
    } else {
        prefix.join("bin").join("tif")
    };
    if bin.is_file() {
        return Ok(bin);
    }
    Err(format!(
        "could not locate installed tif under {}",
        prefix.display()
    ))
}

fn uninstall_prefix(prefix: &Path, home: &Path, purge_secrets: bool) -> Result<(), String> {
    let root = workspace_root();
    let status = if cfg!(windows) {
        let mut args = vec![
            "-NoProfile".into(),
            "-ExecutionPolicy".into(),
            "Bypass".into(),
            "-File".into(),
            root.join("scripts/uninstall.ps1").display().to_string(),
            "-Prefix".into(),
            prefix.display().to_string(),
        ];
        if purge_secrets {
            args.push("-PurgeSecrets".into());
        }
        Command::new("powershell")
            .args(&args)
            .env("APPDATA", home.join("AppData").join("Roaming"))
            .env("LOCALAPPDATA", home.join("AppData").join("Local"))
            .env("HOME", home)
            .env("USERPROFILE", home)
            .status()
            .map_err(|e| e.to_string())?
    } else {
        let bash = find_bash().ok_or("bash required for uninstall.sh")?;
        let script = to_bash_path(&root.join("scripts/uninstall.sh"));
        let pref = to_bash_path(prefix);
        let mut args = vec![script.clone(), "--prefix".into(), pref];
        if purge_secrets {
            args.push("--purge-secrets".into());
        }
        Command::new(bash)
            .args(&args)
            .env("HOME", home)
            .env("XDG_CONFIG_HOME", home.join(".config"))
            .status()
            .map_err(|e| e.to_string())?
    };
    if !status.success() {
        return Err(format!("uninstall failed: {status:?}"));
    }
    Ok(())
}

fn run_bin_version(bin: &Path) -> Result<String, String> {
    let out = Command::new(bin)
        .arg("--version")
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(format!(
            "version failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn find_bash() -> Option<PathBuf> {
    for name in ["bash", "bash.exe"] {
        if let Ok(out) = Command::new(name).arg("--version").output() {
            if out.status.success() {
                return Some(PathBuf::from(name));
            }
        }
    }
    let git_bash = PathBuf::from(r"C:\Program Files\Git\bin\bash.exe");
    if git_bash.is_file() {
        Some(git_bash)
    } else {
        None
    }
}

fn to_bash_path(p: &Path) -> String {
    let s = p
        .canonicalize()
        .unwrap_or_else(|_| p.to_path_buf())
        .display()
        .to_string();
    let s = s
        .strip_prefix(r"\\?\")
        .or_else(|| s.strip_prefix("//?/"))
        .unwrap_or(&s)
        .replace('\\', "/");
    if s.len() >= 2 && s.as_bytes().get(1) == Some(&b':') {
        let drive = s.chars().next().unwrap().to_ascii_lowercase();
        format!("/{drive}{}", &s[2..])
    } else {
        s
    }
}
