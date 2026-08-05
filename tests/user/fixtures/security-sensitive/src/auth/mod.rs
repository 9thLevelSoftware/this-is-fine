//! Auth surface under sensitive_paths.

mod login;

pub use login::{login, validate_password};
