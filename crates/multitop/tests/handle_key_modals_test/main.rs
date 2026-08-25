//! Key dispatch while a modal owns the screen.
//!
//! Every one of these is a case where the wrong key doing the wrong thing has
//! a real cost: a stray `Enter` that kills a running dpkg on N hosts, a key
//! swallowed while a biometric prompt hangs so the app can only be killed, a
//! password field that silently eats what was typed into it.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use multitop::app::{App, Confirm, Msg, VaultState};
use multitop::config::Server;
use multitop::panel::UpgradeState;
use multitop::password_store;
use multitop::run::{handle_key, Tasks};
use tokio::sync::{mpsc, watch};

fn test_server(host: &str) -> Server {
    Server {
        host: host.to_string(),
        port: 0,
        user: "admin".to_string(),
        upgrade_cmd: Some("true".to_string()),
    }
}

async fn isolate() -> tokio::sync::MutexGuard<'static, ()> {
    let guard = password_store::lock_for_test_async().await;
    password_store::enable_mock_store();
    password_store::clear_mock_store();
    guard
}

/// Everything `handle_key` needs besides the app, and nothing that reaches the
/// network: the channel is drained by the test, not by a task.
struct Keys {
    tx: mpsc::Sender<Msg>,
    rx: mpsc::Receiver<Msg>,
    dims_rx: Arc<watch::Receiver<(u16, u16)>>,
    tasks: Tasks,
}

impl Keys {
    fn new(panels: usize) -> Self {
        let (tx, rx) = mpsc::channel::<Msg>(64);
        let (dims_tx, dims_rx) = watch::channel((80, 24));
        // Kept alive so the receiver stays valid for the whole test.
        std::mem::forget(dims_tx);
        Self {
            tx,
            rx,
            dims_rx: Arc::new(dims_rx),
            tasks: Tasks::new(panels),
        }
    }

    fn press(&mut self, app: &mut App, code: KeyCode) {
        self.press_with(app, code, KeyModifiers::NONE);
    }

    fn press_with(&mut self, app: &mut App, code: KeyCode, modifiers: KeyModifiers) {
        handle_key(
            KeyEvent::new_with_kind(code, modifiers, KeyEventKind::Press),
            app,
            (80, 24),
            self.dims_rx.clone(),
            &self.tx,
            &mut self.tasks,
        );
    }

    fn release(&mut self, app: &mut App, code: KeyCode) {
        handle_key(
            KeyEvent::new_with_kind(code, KeyModifiers::NONE, KeyEventKind::Release),
            app,
            (80, 24),
            self.dims_rx.clone(),
            &self.tx,
            &mut self.tasks,
        );
    }

    fn type_str(&mut self, app: &mut App, s: &str) {
        for c in s.chars() {
            self.press(app, KeyCode::Char(c));
        }
    }
}

fn app_with_config(dir: &tempfile::TempDir, hosts: &[&str]) -> App {
    let mut app = App::new(hosts.iter().map(|h| test_server(h)).collect());
    app.config_path = Some(dir.path().join("config.toml"));
    app
}

// ---------------------------------------------------------------- key releases
mod quit_and_upgrade_confirm;
mod sort_and_paging;
mod vault_prompts;
