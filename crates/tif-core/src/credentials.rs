//! Resolve local credential references for reviewer backends.
//!
//! Supported forms:
//! - `ENV_NAME` or `env:ENV_NAME` — read from process environment
//! - `file:PATH` — read trimmed contents of a file (owner-readable secret file)
//!
//! Credentials are never written to audit logs by this module.

use std::env;
use std::fs;
use std::path::Path;

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

fn read_secret_file(path: &Path) -> Result<String> {
    let raw = fs::read_to_string(path).map_err(|e| {
        TifError::Config(format!(
            "failed to read credential file {}: {e}",
            path.display()
        ))
    })?;
    let secret = raw.trim().to_string();
    if secret.is_empty() {
        return Err(TifError::Config(format!(
            "credential file {} is empty",
            path.display()
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
    use tempfile::NamedTempFile;

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
    fn resolves_file() {
        let f = NamedTempFile::new().unwrap();
        std::fs::write(f.path(), "  file-secret\n").unwrap();
        let ref_s = format!("file:{}", f.path().display());
        let v = resolve_credential(&ref_s).unwrap();
        assert_eq!(v, "file-secret");
    }

    #[test]
    fn rejects_path_shaped_env_name() {
        assert!(resolve_credential("env:C:/secrets/key").is_err());
    }
}
