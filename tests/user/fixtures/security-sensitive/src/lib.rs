//! Security-sensitive fixture root.

pub mod auth;

pub use auth::{login, validate_password};
