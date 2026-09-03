//! End-to-end coverage for the Configuration panel, keystroke through render.
//!
//! Written after a report that entering a password in Server Settings crashed
//! the app. These do **not** reproduce that crash -- see `docs/roadmap.md` for
//! what was ruled out -- but they close the hole that let it be reported by a
//! person rather than by the suite.
//!
//! The hole: every piece of that flow already had a test -- `handle_key`
//! returned the right `PasswordAction`, `apply` wrote to the right store -- and
//! not one of them ran the renderer. A panic in `config_ui::draw` was
//! invisible to the whole suite. Testing state transitions without drawing the
//! frame they produce leaves half the app untested, and it is the half the user
//! sees.
//!
//! So these drive the real path: real `KeyEvent`s through `run::handle_key`,
//! real `password_actions::apply`, and a real `ui::draw` into a `TestBackend`
//! after *every* press. A panic anywhere in that chain fails the test.
//!
//! The sweep at the bottom is the structural part. Rather than pinning one
//! reported sequence, it walks every key sequence the panel accepts up to a
//! fixed depth and draws each resulting frame, so an unrenderable state fails
//! here instead of on somebody's terminal.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use ratatui::backend::TestBackend;
use ratatui::Terminal;
use tokio::sync::{mpsc, watch};

use multitop::app::{App, Msg};
use multitop::config::Server;
use multitop::password_store;
use multitop::run::{handle_key, Tasks};

fn server(host: &str) -> Server {
    Server {
        host: host.to_string(),
        port: 22,
        user: "admin".to_string(),
        upgrade_cmd: Some("ls -l ; ls -l".to_string()),
        custom_command: None,
    }
}

/// Keep the sweep away from the user's real `~/.ssh/config`.
///
/// `I` in the Servers section reads it and merges what it finds into the
/// configuration. Pointed at the real file, this test would import the
/// developer's hosts and its result would depend on whose machine it ran on.
fn isolate_ssh_config() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let path = std::env::temp_dir().join(format!("mt_e2e_ssh_config_{}", std::process::id()));
        std::fs::write(
            &path,
            "Host sweep-import\n    HostName 10.0.0.1\n    User admin\n",
        )
        .unwrap();
        std::env::set_var("MULTITOP_SSH_CONFIG", &path);
    });
}

struct Harness {
    app: App,
    servers: Vec<Server>,
    tasks: Tasks,
    tx: mpsc::Sender<Msg>,
    /// Kept, not dropped: the vault work a key press spawns reports back
    /// through this channel, and a dropped receiver means every one of those
    /// results silently disappears -- so a test could never see what the app
    /// did with them.
    rx: mpsc::Receiver<Msg>,
    dims_rx: Arc<watch::Receiver<(u16, u16)>>,
    terminal: Terminal<TestBackend>,
    /// Mirrors the event loop's `known_epoch`, so the harness refreshes
    /// `servers` and resizes `tasks` at the same moment `run` does -- after the
    /// press, never during it.
    known_epoch: u64,
    _dir: tempfile::TempDir,
}

impl Harness {
    fn new(hosts: &[&str]) -> Self {
        isolate_ssh_config();
        let servers: Vec<Server> = hosts.iter().map(|h| server(h)).collect();
        let mut app = App::new(servers.clone());
        let dir = tempfile::tempdir().unwrap();
        app.config_path = Some(dir.path().join("config.toml"));
        let (tx, rx) = mpsc::channel::<Msg>(256);
        let (dims_tx, drx) = watch::channel((80u16, 24u16));
        drop(dims_tx);
        let known_epoch = app.panels_epoch;
        Self {
            tasks: Tasks::new(servers.len()),
            terminal: Terminal::new(TestBackend::new(80, 24)).unwrap(),
            app,
            servers,
            tx,
            rx,
            dims_rx: Arc::new(drx),
            known_epoch,
            _dir: dir,
        }
    }

    fn press(&mut self, code: KeyCode) {
        handle_key(
            KeyEvent {
                code,
                modifiers: KeyModifiers::NONE,
                kind: KeyEventKind::Press,
                state: KeyEventState::NONE,
            },
            &mut self.app,
            (80, 24),
            Arc::clone(&self.dims_rx),
            &self.tx,
            &mut self.tasks,
        );
        // What `run`'s event loop does after a press that edited the server
        // list. `Tasks::fit_to` and `restart_all_agents` are private, so the
        // sizing is reproduced with a fresh `Tasks` of the right length -- the
        // property that matters here is that during the press itself, `servers`
        // and `tasks` are still the previous list's size.
        if self.app.panels_epoch != self.known_epoch {
            self.known_epoch = self.app.panels_epoch;
            self.servers = self.app.panels.iter().map(|p| p.server.clone()).collect();
            self.tasks = Tasks::new(self.servers.len());
        }
        self.draw();
    }

    fn type_str(&mut self, text: &str) {
        for c in text.chars() {
            self.press(KeyCode::Char(c));
        }
    }

    /// Deliver every message the spawned work has produced, exactly as the
    /// event loop does, and return how many there were.
    ///
    /// `CredentialLoaded` messages do not count: they are the background answer
    /// to the store lookup the password manager dispatches on open, not a user
    /// action or decision, and the parity assertions built on this count mean
    /// "no duplicate vault attempt", never "exactly one message total".
    ///
    /// The wait is bounded: one attempt that produces nothing is the answer to
    /// "did a second attempt start?", so it must not hang waiting for one.
    async fn pump(&mut self, patience: std::time::Duration) -> usize {
        let mut delivered = 0;
        // A duplicate attempt is queued behind the first on this runtime, so
        // the grace has to cover one more Argon2id run at test parameters
        // (tens of milliseconds) with room to spare -- otherwise "no second
        // attempt" would just mean "did not wait for it".
        let grace = std::time::Duration::from_secs(3);
        loop {
            let wait = if delivered == 0 { patience } else { grace };
            let Ok(Some(msg)) = tokio::time::timeout(wait, self.rx.recv()).await else {
                break;
            };
            if !matches!(&msg, Msg::CredentialLoaded { .. }) {
                delivered += 1;
            }
            self.app.apply(msg);
        }
        self.draw();
        delivered
    }

    /// Render the frame this state produces. The assertion is that it returns.
    fn draw(&mut self) {
        self.terminal
            .draw(|f| multitop::ui::draw(f, &mut self.app))
            .unwrap();
    }

    /// The buffer as text.
    ///
    /// Rows are split on the buffer's OWN width. This was hardcoded to 80, so
    /// the moment a test resized the terminal the rows were reassembled at the
    /// wrong stride and every assertion after it was reading scrambled text --
    /// a harness that lies about the product, which is worse than no harness.
    fn screen(&self) -> String {
        let buf = self.terminal.backend().buffer();
        let width = buf.area.width as usize;
        buf.content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect::<Vec<_>>()
            .chunks(width)
            .map(<[&str]>::concat)
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn notice(&self) -> String {
        self.app
            .password_manager
            .as_ref()
            .and_then(|m| m.notice.clone())
            .unwrap_or_default()
    }
}

async fn setup() -> tokio::sync::MutexGuard<'static, ()> {
    let guard = password_store::lock_for_test_async().await;
    password_store::enable_mock_store();
    password_store::clear_mock_store();
    guard
}

// ---------------------------------------------------------------------------
// One list, one row editor. A password belongs to the host it is for.
// ---------------------------------------------------------------------------

mod password_editor;
mod render_and_sweep;
mod vault_creation;
