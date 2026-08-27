//! The two-press `u` flow.
//!
//! Pressing `u` once switches into the Upgrade view and does nothing else;
//! pressing it again starts the run. These tests drive the real key handler,
//! because the property being protected is a *sequence* — testing the `App`
//! methods individually would not catch the sequence regressing.
//!
//! The rule this pins down: the behaviour of the first press must not depend on
//! whether an upgrade has ever run. It used to, which is how `u` could jump
//! straight to the confirm modal on a fresh start but show the pane once an
//! upgrade had happened.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use tokio::sync::{mpsc, watch};

use multitop::app::{App, Mode, Msg};
use multitop::config::Server;
use multitop::run::{handle_key, Tasks};
use multitop::state::HostUpdate;

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

fn server(host: &str, cmd: Option<&str>) -> Server {
    Server {
        host: host.to_string(),
        port: 22,
        user: "admin".to_string(),
        upgrade_cmd: cmd.map(str::to_string),
    }
}

struct Harness {
    app: App,
    servers: Vec<Server>,
    tasks: Tasks,
    tx: mpsc::Sender<Msg>,
    /// Messages the key handler emitted, so a test can assert that a press
    /// which should be inert really did not queue any work.
    rx: mpsc::Receiver<Msg>,
    /// Messages a settle saw from a producer it does not own; kept for
    /// `emitted()` so a press never eats another producer's report.
    pending: std::collections::VecDeque<Msg>,
    dims_rx: Arc<watch::Receiver<(u16, u16)>>,
}

impl Harness {
    fn new(servers: Vec<Server>) -> Self {
        let app = App::new(servers.clone());
        let (tx, rx) = mpsc::channel::<Msg>(64);
        let (dims_tx, drx) = watch::channel((80u16, 24u16));
        // The receiver keeps working after the sender goes; nothing here resizes.
        drop(dims_tx);
        Self {
            tasks: Tasks::new(servers.len()),
            app,
            servers,
            tx,
            rx,
            pending: std::collections::VecDeque::new(),
            dims_rx: Arc::new(drx),
        }
    }

    fn press(&mut self, c: char) {
        self.press_key(KeyCode::Char(c));
    }

    fn press_key(&mut self, code: KeyCode) {
        handle_key(
            KeyEvent {
                code,
                modifiers: KeyModifiers::NONE,
                kind: KeyEventKind::Press,
                state: crossterm::event::KeyEventState::NONE,
            },
            &mut self.app,
            (80, 24),
            Arc::clone(&self.dims_rx),
            &self.tx,
            &mut self.tasks,
        );
        // The real loop applies a worker's answer before the next key is read;
        // this harness hands keys straight to `handle_key`, so it reproduces
        // that step itself for the store lookups the enter/open keys dispatch.
        // Without it the confirm stays deferred on a lookup that has already
        // answered, and the first press would race a message it can see but
        // never act on.
        for _ in 0..25 {
            if self.app.panels.iter().all(|p| !p.password_checking) {
                return;
            }
            if let Ok(m) = self.rx.try_recv() {
                if matches!(m, Msg::CredentialLoaded { .. }) {
                    self.app.apply(m);
                    return;
                }
                // Not a credential answer: keep it for `emitted`.
                self.pending.push_back(m);
            } else {
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
        }
    }

    /// Messages emitted so far, without blocking.
    fn emitted(&mut self) -> Vec<Msg> {
        let mut out = std::mem::take(&mut self.pending);
        while let Ok(m) = self.rx.try_recv() {
            out.push_back(m);
        }
        out.into()
    }

    fn pane_text(&self, panel: usize) -> String {
        strip_ansi(
            &multitop::ui::pane_lines(&self.app, panel, usize::MAX, 0, 0)
                .0
                .join("\n"),
        )
    }
}

fn strip_ansi(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            for c2 in chars.by_ref() {
                if c2 == 'm' {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// 1. First press switches to the pane, and starts nothing.
// ---------------------------------------------------------------------------

mod confirm_modal_filter_quit;
mod keybar_rows;
mod pane_entry;
mod while_running;

/// The upgrade generation the panel is currently running.
fn upgrade_gen(h: &Harness, panel: usize) -> u64 {
    h.app.panels[panel].upgrade_gen
}

fn start_upgrade(h: &mut Harness) {
    h.press('u');
    h.press('u');
    h.press('u');
    assert!(h.app.upgrades_in_flight(), "precondition: upgrade started");
}

/// Rendered text of the keybar row, whatever it is showing right now.
fn keybar_text(app: &App, width: u16) -> String {
    let theme = multitop_agent::color::ANSI;
    multitop::ui::keybar_content(app, &theme, width, Mode::Monitor)
        .spans
        .iter()
        .map(|s| s.content.as_ref())
        .collect()
}
