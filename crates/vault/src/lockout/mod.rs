//! Unlock rate limiting: how many attempts, how long the wait, and where the
//! count is kept so that deleting a file cannot reset it.

mod guard;
pub(crate) mod state;

#[cfg(test)]
#[path = "kill_resistance_tests.rs"]
mod kill_resistance_tests;
#[cfg(test)]
#[path = "lockout_tests.rs"]
mod lockout_tests;
#[cfg(test)]
#[path = "write_ahead_tests.rs"]
mod write_ahead_tests;

pub use guard::LockoutGuard;
pub use state::LockoutState;
