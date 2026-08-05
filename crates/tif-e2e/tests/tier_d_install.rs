//! Tier D — install / checksum contract (UT-4).
//! Exercises the **real** verifier in `scripts/lib/sha256-verify.sh` (shared with install.sh).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tif_e2e::{assert_scenario, run_scenario, workspace_root};

#[test]
fn d01_install_sums_match() {
    let r = run_scenario("D01", || {
        let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
        let asset_name = "tif-x86_64-unknown-linux-gnu.tar.gz";
        let asset = dir.path().join(asset_name);
        let body = b"fake-release-payload-for-checksum";
        fs::write(&asset, body).map_err(|e| e.to_string())?;

        let digest = real_sha256_hex(&asset)?;
        let sums = dir.path().join("SHA256SUMS");
        fs::write(&sums, format!("{digest}  {asset_name}\n")).map_err(|e| e.to_string())?;

        run_sha256_verify(&asset, asset_name, &sums)?;
        Ok(format!("real verifier match ok digest={digest}"))
    });
    assert_scenario(&r);
}

#[test]
fn d02_install_refuse_mismatch_and_missing() {
    let r = run_scenario("D02", || {
        let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
        let asset_name = "tif-x86_64-unknown-linux-gnu.tar.gz";
        let asset = dir.path().join(asset_name);
        fs::write(&asset, b"payload-a").map_err(|e| e.to_string())?;

        // Mismatch
        let sums_bad = dir.path().join("SHA256SUMS.bad");
        fs::write(
            &sums_bad,
            "0000000000000000000000000000000000000000000000000000000000000000  tif-x86_64-unknown-linux-gnu.tar.gz\n",
        )
        .map_err(|e| e.to_string())?;
        match run_sha256_verify(&asset, asset_name, &sums_bad) {
            Ok(()) => return Err("mismatch must refuse".into()),
            Err(e) if e.to_lowercase().contains("mismatch") => {}
            Err(e) => return Err(format!("expected mismatch error, got: {e}")),
        }

        // Missing entry
        let sums_miss = dir.path().join("SHA256SUMS.miss");
        fs::write(
            &sums_miss,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa  other-asset.bin\n",
        )
        .map_err(|e| e.to_string())?;
        match run_sha256_verify(&asset, asset_name, &sums_miss) {
            Ok(()) => return Err("missing entry must refuse".into()),
            Err(e) if e.to_lowercase().contains("not listed") => {}
            Err(e) => return Err(format!("expected not-listed error, got: {e}")),
        }

        // Missing SUMS file
        let missing = dir.path().join("nope-SUMS");
        match run_sha256_verify(&asset, asset_name, &missing) {
            Ok(()) => return Err("missing SUMS file must refuse".into()),
            Err(e) => {
                let l = e.to_lowercase();
                if !(l.contains("sums") || l.contains("not found") || l.contains("no such")) {
                    return Err(format!("expected missing-sums error, got: {e}"));
                }
            }
        }

        Ok("real verifier: mismatch + missing entry + missing file all refuse".into())
    });
    assert_scenario(&r);
}

fn verify_script() -> PathBuf {
    workspace_root()
        .join("scripts")
        .join("lib")
        .join("sha256-verify.sh")
}

/// Invoke the production shell verifier (same code install.sh sources).
fn run_sha256_verify(asset: &Path, asset_name: &str, sums: &Path) -> Result<(), String> {
    let script = verify_script();
    if !script.is_file() {
        return Err(format!("missing verifier script {}", script.display()));
    }
    let bash = find_bash().ok_or_else(|| {
        "bash not found (required to exercise scripts/lib/sha256-verify.sh)".to_string()
    })?;
    let out = Command::new(&bash)
        .arg(to_bash_path(&script))
        .arg(to_bash_path(asset))
        .arg(asset_name)
        .arg(to_bash_path(sums))
        .output()
        .map_err(|e| format!("spawn bash verifier: {e}"))?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    if out.status.success() {
        Ok(())
    } else {
        Err(format!(
            "verifier failed status={:?}: {stdout}{stderr}",
            out.status.code()
        ))
    }
}

fn real_sha256_hex(path: &Path) -> Result<String, String> {
    // Use the same shell helper as the installer for digest generation.
    let bash = find_bash().ok_or("bash not found")?;
    let script = verify_script();
    let out = Command::new(&bash)
        .args([
            "-c",
            &format!(
                "source '{}' && sha256_file '{}'",
                to_bash_path(&script),
                to_bash_path(path)
            ),
        ])
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(format!(
            "sha256_file failed: {}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    // Keep only hex digits (MSYS can prefix odd escapes on some paths).
    let raw = String::from_utf8_lossy(&out.stdout);
    let hex: String = raw.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    if hex.len() != 64 {
        return Err(format!("unexpected digest from sha256_file: {raw:?}"));
    }
    Ok(hex.to_ascii_lowercase())
}

/// Convert a Windows path to a Git-Bash friendly `/d/foo` form when needed.
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
    // `D:/path` → `/d/path` for MSYS/Git Bash.
    if s.len() >= 2 && s.as_bytes().get(1) == Some(&b':') {
        let drive = s.chars().next().unwrap().to_ascii_lowercase();
        format!("/{drive}{}", &s[2..])
    } else {
        s
    }
}

fn find_bash() -> Option<PathBuf> {
    for name in ["bash", "bash.exe"] {
        if let Ok(out) = Command::new(name).arg("--version").output() {
            if out.status.success() {
                return Some(PathBuf::from(name));
            }
        }
    }
    // Common Git for Windows path
    let git_bash = PathBuf::from(r"C:\Program Files\Git\bin\bash.exe");
    if git_bash.is_file() {
        return Some(git_bash);
    }
    None
}
