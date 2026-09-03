//! Automated E2E Integration Tests for the Full Upgrade Execution Loop
//!
//! Validates the complete upgrade flow:
//! 1. `spawn_upgrade` spawns local processes, streams output via Msg channel
//! 2. App state machine correctly processes AuxBegin/AuxLine/AuxDone messages
//! 3. Output collection, carriage return cleaning, exit status reporting
//!
//! Local tests run automatically; remote tests are `#[ignore]`d.
//!
//! Run local tests: `cargo test --test upgrade_loop_e2e`
//! Run remote tests: `cargo test --test upgrade_loop_remote_e2e -- --ignored`

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::time::Duration;

use multitop::app::{App, Msg, VaultState};
use multitop::config::Server;
use multitop::panel::{Mode, UpgradeState};
use multitop::password_store;
use multitop::tasks::spawn_upgrade;
use multitop::types::Command;

/// Press a key the way the app does — through the real dispatcher.
///
/// Several tests here used to re-implement the `u` handler's decision chain
/// inline ("replicate the key handler decision"), and the chain they wrote had
/// a branch the handler does not have. A test that models the code under test
/// passes whatever the code does.
fn press(app: &mut App, code: crossterm::event::KeyCode) {
    use crossterm::event::{KeyEvent, KeyEventKind, KeyModifiers};
    let (tx, _rx) = mpsc::channel::<Msg>(64);
    let (dims_tx, dims_rx) = tokio::sync::watch::channel((80u16, 24u16));
    std::mem::forget(dims_tx);
    let mut tasks = multitop::run::Tasks::new(app.panels.len());
    multitop::run::handle_key(
        KeyEvent::new_with_kind(code, KeyModifiers::NONE, KeyEventKind::Press),
        app,
        (80, 24),
        std::sync::Arc::new(dims_rx),
        &tx,
        &mut tasks,
    );
}

use tokio::sync::mpsc;

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

/// Test helper: create a local Server (127.0.0.1 triggers local shell path).
fn local_server(upgrade_cmd: &str) -> Server {
    Server {
        host: "127.0.0.1".to_string(),
        port: 0,
        user: "testuser".to_string(),
        upgrade_cmd: Some(upgrade_cmd.to_string()),
        custom_command: None,
    }
}

/// Enable mock password store for tests (auto-enabled via cfg!(test) but explicit for clarity).
/// Reset the process-global mock store, holding the test guard so a
/// concurrently running test cannot be wiped out mid-run. Keep the returned
/// guard alive for the whole test body.
async fn enable_test_mock_store() -> tokio::sync::MutexGuard<'static, ()> {
    let guard = password_store::lock_for_test_async().await;
    password_store::enable_mock_store();
    password_store::clear_mock_store();
    guard
}

/// The same, for `#[test]` bodies that cannot await.
///
/// These tests drive the real `enter_upgrade_view`, which loads saved passwords
/// so it can tell the user truthfully whether a prompt is coming. Without this
/// guard that load reaches the OS keychain from the test suite.
fn enable_test_mock_store_blocking() -> tokio::sync::MutexGuard<'static, ()> {
    let guard = password_store::lock_for_test();
    password_store::enable_mock_store();
    password_store::clear_mock_store();
    guard
}

/// Test helper: collect messages from channel with timeout.
struct MsgCollector {
    rx: mpsc::Receiver<Msg>,
}

impl MsgCollector {
    const fn new(rx: mpsc::Receiver<Msg>) -> Self {
        Self { rx }
    }

    async fn collect_all(&mut self) -> Vec<Msg> {
        let mut msgs = Vec::new();
        while let Ok(Some(msg)) = tokio::time::timeout(Duration::from_secs(5), self.rx.recv()).await
        {
            msgs.push(msg);
        }
        msgs
    }

    async fn wait_for_done(&mut self) -> Option<Msg> {
        loop {
            match tokio::time::timeout(Duration::from_secs(10), self.rx.recv()).await {
                Ok(Some(msg)) => {
                    if matches!(msg, Msg::AuxDone { .. } | Msg::Status { .. }) {
                        return Some(msg);
                    }
                }
                _ => return None,
            }
        }
    }
}

mod no_upgrade_cmd_regression;
mod phase1_stream;
mod phase3_security;
mod ui_cycle;
