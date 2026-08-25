//! Coverage tests for pure functions and state machine paths that the
//! integration tests don't reach. Each test exercises a specific uncovered
//! function or code path through the PUBLIC API.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod config_ui;
mod panel;
mod text_and_state;
