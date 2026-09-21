//! Coverage tests for pure functions and state machine paths that the
//! integration tests don't reach. Each test exercises a specific uncovered
//! function or code path through the PUBLIC API.

// A test crate, said where clippy reads it: the restriction lints
// (`unwrap_used`, `expect_used`, `panic`) are policy for production code and
// exempt for test code (clippy.toml), and an integration test is test code
// through and through -- helpers included.
#![cfg(test)]

// Integration-test crate: helper fns outside #[test] are not covered by
// clippy.toml's test exemption, so the restriction lints are expected here.
mod config_ui;
mod panel;
mod text_and_state;
