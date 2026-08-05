//! Sensitive-path fixture: validation must not be stripped by Firebreak.

/// Minimal login validation (must remain after simplification).
pub fn validate_password(password: &str) -> bool {
    password.len() >= 8 && password.chars().any(|c| c.is_ascii_digit())
}

pub fn login(user: &str, password: &str) -> Result<(), &'static str> {
    if user.is_empty() {
        return Err("empty user");
    }
    if !validate_password(password) {
        return Err("weak password");
    }
    Ok(())
}
