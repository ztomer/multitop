//! Password and server management: types, actions, tests.

mod actions;
mod types;

#[cfg(test)]
#[path = "passwords_tests.rs"]
mod password_unit_tests;

pub use actions::{handle_key, open};
pub use types::{PasswordAction, PasswordEdit, PasswordManager, ServerDraft};
