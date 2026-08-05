//! Resolve local credential references for reviewer backends.
//!
//! Supported forms:
//! - `ENV_NAME` or `env:ENV_NAME` — read from process environment
//! - `file:PATH` — read trimmed contents of a file under the allowlisted secrets dir
//!
//! `file:` paths must resolve (after canonicalize) under:
//! - Unix: `~/.config/tif/secrets/` (or `$XDG_CONFIG_HOME/tif/secrets/`)
//! - Windows: `%APPDATA%/tif/secrets/`
//!
//! Credentials are never written to audit logs by this module.

use std::env;
use std::fs;
use std::path::{Component, Path, PathBuf};

use crate::error::{Result, TifError};

/// Resolve a credential reference to a secret string.
pub fn resolve_credential(credential_ref: &str) -> Result<String> {
    let trimmed = credential_ref.trim();
    if trimmed.is_empty() {
        return Err(TifError::Config(
            "empty credential_ref; set env:VAR or file:PATH".into(),
        ));
    }

    if let Some(path) = trimmed.strip_prefix("file:") {
        let path = path.trim();
        if path.is_empty() {
            return Err(TifError::Config(
                "credential_ref file: path is empty".into(),
            ));
        }
        return read_secret_file(Path::new(path));
    }

    let env_name = trimmed.strip_prefix("env:").unwrap_or(trimmed).trim();
    if env_name.is_empty() {
        return Err(TifError::Config("credential_ref env name is empty".into()));
    }
    // Reject path separators in env names (common footgun).
    if env_name.contains('/') || env_name.contains('\\') {
        return Err(TifError::Config(format!(
            "credential_ref `{env_name}` looks like a path; use file:PATH for files"
        )));
    }

    env::var(env_name).map_err(|_| {
        TifError::Config(format!(
            "credential environment variable `{env_name}` is not set"
        ))
    })
}

/// Platform secrets directory: `~/.config/tif/secrets` or `%APPDATA%/tif/secrets`.
pub fn secrets_dir() -> Result<PathBuf> {
    let base = directories::ProjectDirs::from("", "", "tif")
        .map(|d| d.config_dir().to_path_buf())
        .or_else(|| {
            // Fallback when directories cannot resolve (rare): HOME/.config/tif
            env::var_os("HOME")
                .or_else(|| env::var_os("USERPROFILE"))
                .map(|h| PathBuf::from(h).join(".config").join("tif"))
        })
        .ok_or_else(|| {
            TifError::Config(
                "cannot resolve tif config dir for credentials (no home/appdata)".into(),
            )
        })?;
    Ok(base.join("secrets"))
}

fn reject_path_traversal_components(path: &Path) -> Result<()> {
    for c in path.components() {
        match c {
            Component::ParentDir => {
                return Err(TifError::Config(
                    "credential file: path must not contain `..` components".into(),
                ));
            }
            Component::CurDir => {
                return Err(TifError::Config(
                    "credential file: path must not contain `.` components".into(),
                ));
            }
            _ => {}
        }
    }
    Ok(())
}

fn path_is_under(child: &Path, parent: &Path) -> bool {
    let mut c = child.components();
    for p in parent.components() {
        match c.next() {
            Some(cc) if cc == p => {}
            _ => return false,
        }
    }
    true
}

fn read_secret_file(path: &Path) -> Result<String> {
    reject_path_traversal_components(path)?;

    let allowed = secrets_dir()?;
    // Ensure the secrets root exists so canonicalize of the dir works when we check.
    fs::create_dir_all(&allowed).map_err(|e| {
        TifError::Config(format!(
            "cannot create secrets dir {}: {e}",
            allowed.display()
        ))
    })?;
    let allowed_canon = allowed.canonicalize().map_err(|e| {
        TifError::Config(format!(
            "cannot canonicalize secrets dir {}: {e}",
            allowed.display()
        ))
    })?;

    // Resolve relative paths against the secrets dir; absolute paths must still land under it.
    let candidate = if path.is_absolute() {
        path.to_path_buf()
    } else {
        allowed.join(path)
    };

    // Lexical containment before canonicalize (defense in depth for non-existent files).
    // After join, strip `..` via components already rejected; still check prefix when absolute.
    if path.is_absolute() {
        // Compare normalized absolute without requiring the file to exist yet.
        let mut norm = PathBuf::new();
        for c in candidate.components() {
            match c {
                Component::ParentDir => {
                    return Err(TifError::Config(
                        "credential file: path escapes secrets directory".into(),
                    ));
                }
                Component::CurDir => {}
                other => norm.push(other),
            }
        }
        if !path_is_under(&norm, &allowed_canon) && !path_is_under(&norm, &allowed) {
            // Also allow if norm will canonicalize under allowed — checked below if file exists.
            // Reject obvious escapes early when neither parent matches.
            let allowed_s = allowed_canon.to_string_lossy().to_ascii_lowercase();
            let norm_s = norm.to_string_lossy().to_ascii_lowercase();
            if !norm_s.starts_with(allowed_s.trim_end_matches(['/', '\\'])) {
                return Err(TifError::Config(format!(
                    "credential file: path must be under secrets dir {} (got {})",
                    allowed.display(),
                    path.display()
                )));
            }
        }
    }

    let canon = candidate.canonicalize().map_err(|e| {
        TifError::Config(format!(
            "failed to resolve credential file {} (must exist under {}): {e}",
            path.display(),
            allowed.display()
        ))
    })?;

    if !path_is_under(&canon, &allowed_canon) {
        return Err(TifError::Config(format!(
            "credential file: resolved path {} is outside allowlisted secrets dir {}",
            canon.display(),
            allowed_canon.display()
        )));
    }

    let raw = fs::read_to_string(&canon).map_err(|e| {
        TifError::Config(format!(
            "failed to read credential file {}: {e}",
            canon.display()
        ))
    })?;
    let secret = raw.trim().to_string();
    if secret.is_empty() {
        return Err(TifError::Config(format!(
            "credential file {} is empty",
            canon.display()
        )));
    }
    Ok(secret)
}

/// Optional resolve: `None` ref → `Ok(None)`.
pub fn resolve_credential_opt(credential_ref: Option<&str>) -> Result<Option<String>> {
    match credential_ref {
        None => Ok(None),
        Some(r) if r.trim().is_empty() => Ok(None),
        Some(r) => resolve_credential(r).map(Some),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // Serialize env mutations across tests.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn resolves_env_var() {
        let _g = ENV_LOCK.lock().unwrap();
        env::set_var("TIF_TEST_CRED_A", "secret-a");
        let v = resolve_credential("TIF_TEST_CRED_A").unwrap();
        assert_eq!(v, "secret-a");
        let v2 = resolve_credential("env:TIF_TEST_CRED_A").unwrap();
        assert_eq!(v2, "secret-a");
        env::remove_var("TIF_TEST_CRED_A");
    }

    #[test]
    fn missing_env_errors() {
        let _g = ENV_LOCK.lock().unwrap();
        env::remove_var("TIF_TEST_CRED_MISSING_XYZ");
        assert!(resolve_credential("TIF_TEST_CRED_MISSING_XYZ").is_err());
    }

    #[test]
    fn resolves_file_under_secrets_dir() {
        let dir = secrets_dir().unwrap();
        fs::create_dir_all(&dir).unwrap();
        let secret_path = dir.join("tif-test-secret-key");
        fs::write(&secret_path, "  file-secret\n").unwrap();

        // Relative name under secrets dir.
        let v = resolve_credential("file:tif-test-secret-key").unwrap();
        assert_eq!(v, "file-secret");

        // Absolute path still under secrets dir.
        let ref_s = format!("file:{}", secret_path.display());
        let v2 = resolve_credential(&ref_s).unwrap();
        assert_eq!(v2, "file-secret");

        let _ = fs::remove_file(&secret_path);
    }

    #[test]
    fn rejects_file_outside_secrets_dir() {
        let tmp = std::env::temp_dir().join(format!("tif-cred-outside-{}", std::process::id()));
        fs::write(&tmp, "nope\n").unwrap();
        let ref_s = format!("file:{}", tmp.display());
        let err = resolve_credential(&ref_s).unwrap_err().to_string();
        assert!(
            err.contains("outside") || err.contains("must be under") || err.contains("secrets"),
            "unexpected err: {err}"
        );
        let _ = fs::remove_file(&tmp);
    }

    #[test]
    fn rejects_path_traversal_in_file_ref() {
        assert!(resolve_credential("file:../outside").is_err());
        assert!(resolve_credential("file:foo/../../etc/passwd").is_err());
        assert!(resolve_credential("file:./secret").is_err());
    }

    #[test]
    fn rejects_path_shaped_env_name() {
        assert!(resolve_credential("env:C:/secrets/key").is_err());
    }
}
