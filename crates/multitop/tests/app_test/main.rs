// A test crate, said where clippy reads it: the restriction lints
// (`unwrap_used`, `expect_used`, `panic`) are policy for production code and
// exempt for test code (clippy.toml), and an integration test is test code
// through and through -- helpers included.
#![cfg(test)]

// Integration-test crate: helper fns outside #[test] are not covered by
// clippy.toml's test exemption, so the restriction lints are expected here.
use multitop::app::*;
use multitop::config::Server;
use multitop::fmt::{error_line, header_line, status_line};
use multitop::panel::UpgradeState;
use multitop_agent::fetch::FetchSnapshot;

/// Divert credentials to the in-memory store, and hold the process-global guard.
///
/// Driving an `App` reaches `password_store` several calls down, and an
/// integration binary is compiled without `cfg(test)`, so the mock is not in
/// force unless it is asked for. Without this these tests query the real OS
/// keychain: every rebuild changes the binary's code signature, so macOS raises
/// an access dialog and the suite stops until a human dismisses it -- and a test
/// can read, overwrite or delete credentials the user depends on.
fn isolate_keychain() -> tokio::sync::MutexGuard<'static, ()> {
    let guard = multitop::password_store::lock_for_test();
    multitop::password_store::enable_mock_store();
    multitop::password_store::clear_mock_store();
    guard
}

fn servers(n: usize) -> Vec<Server> {
    (0..n)
        .map(|i| Server {
            host: format!("s{i}"),
            port: 22,
            user: String::new(),
            upgrade_cmd: None,
            custom_command: None,
        })
        .collect()
}

fn app(n: usize) -> App {
    App::new(servers(n))
}

fn text(p: &Panel) -> String {
    p.view.join("\n")
}
mod mode_and_frames;
mod scroll_and_cache;
mod state_persistence;
mod upgrade_output;
