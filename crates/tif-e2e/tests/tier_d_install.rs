//! Tier D — install / checksum contract (UT-4). Local, no network.

use sha2::{Digest, Sha256};
use std::fs;
use std::path::Path;
use tif_e2e::{run_scenario, ScenarioResult};

#[test]
fn d01_install_sums_match() {
    let r = run_scenario("D01", || {
        let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
        let asset_name = "tif-x86_64-unknown-linux-gnu.tar.gz";
        let asset = dir.path().join(asset_name);
        let body = b"fake-release-payload-for-checksum";
        fs::write(&asset, body).map_err(|e| e.to_string())?;
        let digest = hex_sha256(body);
        let sums = dir.path().join("SHA256SUMS");
        fs::write(&sums, format!("{digest}  {asset_name}\n")).map_err(|e| e.to_string())?;

        verify_local_asset(&asset, asset_name, &sums)?;
        Ok(format!("checksum match ok digest={digest}"))
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
            format!("{}  {asset_name}\n", hex_sha256(b"other")),
        )
        .map_err(|e| e.to_string())?;
        match verify_local_asset(&asset, asset_name, &sums_bad) {
            Ok(()) => return Err("mismatch must refuse".into()),
            Err(e) if e.contains("mismatch") => {}
            Err(e) => return Err(format!("expected mismatch error, got: {e}")),
        }

        // Missing entry
        let sums_miss = dir.path().join("SHA256SUMS.miss");
        fs::write(
            &sums_miss,
            format!("{}  other-asset.bin\n", hex_sha256(b"x")),
        )
        .map_err(|e| e.to_string())?;
        match verify_local_asset(&asset, asset_name, &sums_miss) {
            Ok(()) => return Err("missing entry must refuse".into()),
            Err(e) if e.contains("not listed") => {}
            Err(e) => return Err(format!("expected not-listed error, got: {e}")),
        }

        // Missing SUMS file
        let missing = dir.path().join("nope-SUMS");
        match verify_local_asset(&asset, asset_name, &missing) {
            Ok(()) => return Err("missing SUMS file must refuse".into()),
            Err(e) if e.contains("SUMS") || e.contains("read") || e.contains("not found") => {}
            Err(e) => return Err(format!("expected missing-sums error, got: {e}")),
        }

        Ok("mismatch + missing entry + missing file all refuse".into())
    });
    assert_scenario(&r);
}

/// Local mirror of install.sh `verify_asset` checks (no network).
fn verify_local_asset(asset_path: &Path, asset_name: &str, sums_path: &Path) -> Result<(), String> {
    let sums = fs::read_to_string(sums_path)
        .map_err(|e| format!("could not read SHA256SUMS at {}: {e}", sums_path.display()))?;
    let expected = sums.lines().find_map(|line| {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            return None;
        }
        let mut parts = line.split_whitespace();
        let hex = parts.next()?;
        let name = parts.next()?.trim_start_matches('*');
        if name == asset_name || name.ends_with(asset_name) {
            Some(hex.to_string())
        } else {
            None
        }
    });
    let expected = expected.ok_or_else(|| format!("{asset_name} not listed in SHA256SUMS"))?;
    let bytes = fs::read(asset_path).map_err(|e| e.to_string())?;
    let actual = hex_sha256(&bytes);
    if actual != expected {
        return Err(format!(
            "checksum mismatch for {asset_name}: expected={expected} actual={actual}"
        ));
    }
    Ok(())
}

fn hex_sha256(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    format!("{:x}", h.finalize())
}

fn assert_scenario(r: &ScenarioResult) {
    assert!(
        r.pass,
        "scenario {} failed ({}ms): {}\n{}",
        r.id, r.duration_ms, r.notes, r.log
    );
}
