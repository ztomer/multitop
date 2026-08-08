//! Upgrade view: status header, credential badges, fmt helpers.

mod prod;

#[allow(clippy::module_inception)]
#[cfg(test)]
#[path = "upgrade_view_tests.rs"]
mod tests_module;

pub use prod::{fmt_ago, fmt_duration, header, Credential, Status};
// Re-export for tests
pub use crate::config::Server;
pub use crate::state::{HostUpdate, Outcome};
pub use multitop_agent::color::Palette;
