//! Comprehensive integration tests for Server Settings Manager,
//! keybar visual flare, hotkeys ('e'), and upgrade flow.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use crossterm::event::KeyCode;
use multitop::app::{App, Mode, Msg};
use multitop::config::Server;
use multitop::password_store;
use multitop::passwords::{self, PasswordAction};
use std::sync::atomic::{AtomicU16, Ordering};

/// Divert credentials to the in-memory store, and hold the process-global guard.
///
/// An integration binary is compiled without `cfg(test)`, so the mock store is
/// not in force unless it is asked for, and anything holding an `App` reaches
/// `password_store` several calls down. Without this these tests query the real
/// OS keychain: every rebuild changes the binary's code signature, so macOS
/// raises an access dialog and the suite stops until a human dismisses it.
#[allow(dead_code)]
fn isolate_keychain() -> tokio::sync::MutexGuard<'static, ()> {
    let guard = multitop::password_store::lock_for_test();
    multitop::password_store::enable_mock_store();
    multitop::password_store::clear_mock_store();
    guard
}

#[allow(dead_code)]
async fn isolate_keychain_async() -> tokio::sync::MutexGuard<'static, ()> {
    let guard = multitop::password_store::lock_for_test_async().await;
    multitop::password_store::enable_mock_store();
    multitop::password_store::clear_mock_store();
    guard
}

static PORT_COUNTER: AtomicU16 = AtomicU16::new(10000);

fn next_port() -> u16 {
    PORT_COUNTER.fetch_add(1, Ordering::Relaxed)
}

fn test_server(host: &str) -> Server {
    Server {
        host: host.to_string(),
        port: next_port(),
        user: "admin".to_string(),
        upgrade_cmd: Some("sudo apt update".to_string()),
        custom_command: None,
    }
}

/// Reset the process-global mock store, holding the test guard so a
/// concurrently running test cannot be wiped out mid-run. Keep the returned
/// guard alive for the whole test body.
fn setup_mock_store() -> tokio::sync::MutexGuard<'static, ()> {
    let guard = password_store::lock_for_test();
    reset_store();
    guard
}

/// `setup_mock_store` for `#[tokio::test]` bodies, which must not block the
/// runtime thread to take the guard.
async fn setup_mock_store_async() -> tokio::sync::MutexGuard<'static, ()> {
    let guard = password_store::lock_for_test_async().await;
    reset_store();
    guard
}

fn reset_store() {
    password_store::enable_mock_store();
    password_store::clear_mock_store();
}

mod panel_basics;
mod password_roundtrips;
mod upgrade_and_notices;
