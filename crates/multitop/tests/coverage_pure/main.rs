//! Coverage tests for pure functions and state machine paths that the
//! integration tests don't reach. Each test exercises a specific uncovered
//! function or code path through the PUBLIC API.

// Integration-test crate: helper fns outside #[test] are not covered by
// clippy.toml's test exemption, so the restriction lints are expected here.
mod config_ui;
mod panel;
mod text_and_state;
