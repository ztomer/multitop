//! SSH transport and agent deployment.

mod command;
mod spawn;

#[cfg(test)]
#[path = "ssh_tests.rs"]
mod ssh_tests;

pub use crate::ssh_opts::*;
pub use command::*;
pub use spawn::*;
