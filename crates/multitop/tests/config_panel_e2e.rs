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
        let (tx, _rx) = mpsc::channel::<Msg>(256);
        let (dims_tx, drx) = watch::channel((80u16, 24u16));
        drop(dims_tx);
        let known_epoch = app.panels_epoch;
        Self {
            tasks: Tasks::new(servers.len()),
            terminal: Terminal::new(TestBackend::new(80, 24)).unwrap(),
            app,
            servers,
            tx,
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
            &self.servers,
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

    /// Render the frame this state produces. The assertion is that it returns.
    fn draw(&mut self) {
        self.terminal
            .draw(|f| multitop::ui::draw(f, &self.app))
            .unwrap();
    }

    fn screen(&self) -> String {
        self.terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(ratatui::buffer::Cell::symbol)
            .collect::<Vec<_>>()
            .chunks(80)
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

/// The reported flow, in the shape the panel has now.
#[tokio::test]
async fn a_password_typed_in_the_row_editor_reaches_the_store() {
    let _guard = setup().await;
    let mut h = Harness::new(&["host-a", "host-b"]);

    h.press(KeyCode::Char('e'));
    h.press(KeyCode::Down);
    h.press(KeyCode::Enter);
    assert!(
        h.screen().contains("Editing server"),
        "Enter must open the row editor, got:\n{}",
        h.screen()
    );

    for _ in 0..4 {
        h.press(KeyCode::Tab);
    }
    h.type_str("just-for-b");
    h.press(KeyCode::Enter);

    assert_eq!(
        password_store::load(&server("host-b")).unwrap().as_deref(),
        Some("just-for-b"),
        "it is this host's password"
    );
    assert_eq!(
        password_store::load(&server("host-a")).unwrap(),
        None,
        "and only this host's -- setting one password must not mark others as \
         configured, which is what the shared fallback used to do"
    );
}

/// Adding a server carries its password in the same editor. There is nowhere
/// else to put one.
#[tokio::test]
async fn a_new_server_gets_its_password_in_the_same_editor() {
    let _guard = setup().await;
    let mut h = Harness::new(&["host-a"]);

    h.press(KeyCode::Char('e'));
    h.press(KeyCode::Char('a'));
    h.type_str("host-new");
    h.press(KeyCode::Tab);
    h.type_str("admin");
    // Port is pre-filled with the default; typing into it would append.
    h.press(KeyCode::Tab);
    h.press(KeyCode::Tab);
    h.type_str("ls -l");
    h.press(KeyCode::Tab);
    h.type_str("new-secret");
    h.press(KeyCode::Enter);

    assert_eq!(h.app.panels.len(), 2, "the server must be added");
    assert_eq!(
        password_store::load(&server("host-new"))
            .unwrap()
            .as_deref(),
        Some("new-secret")
    );
}

/// Clearing the field is how a password is taken back, now that the separate
/// Passwords list is gone.
#[tokio::test]
async fn clearing_the_password_field_removes_the_stored_password() {
    let _guard = setup().await;
    let mut h = Harness::new(&["host-a"]);
    password_store::save(&server("host-a"), "old-secret").unwrap();
    h.app.panels[0].sudo_password = Some("old-secret".to_string());
    h.app.panels[0].password_saved = true;

    h.press(KeyCode::Char('e'));
    h.press(KeyCode::Enter);
    for _ in 0..4 {
        h.press(KeyCode::Tab);
    }
    for _ in 0.."old-secret".len() {
        h.press(KeyCode::Backspace);
    }
    h.press(KeyCode::Enter);

    assert_eq!(
        password_store::load(&server("host-a")).unwrap(),
        None,
        "an emptied field must not leave the old password behind"
    );
}

/// Non-ASCII input must survive the round trip through the mask and the buffer.
#[tokio::test]
async fn a_password_with_wide_and_multibyte_characters_renders() {
    let _guard = setup().await;
    let mut h = Harness::new(&["host-a"]);

    h.press(KeyCode::Char('e'));
    h.press(KeyCode::Enter);
    for _ in 0..4 {
        h.press(KeyCode::Tab);
    }
    h.type_str("pä55wörd-\u{4f60}\u{597d}");
    h.press(KeyCode::Backspace);
    h.press(KeyCode::Enter);

    assert_eq!(
        password_store::load(&server("host-a")).unwrap().as_deref(),
        Some("pä55wörd-\u{4f60}")
    );
}

/// The vault offer follows the first stored password, and is part of the flow.
#[tokio::test]
async fn the_vault_creation_offer_renders_and_can_be_declined() {
    let _guard = setup().await;
    let mut h = Harness::new(&["host-a"]);

    h.press(KeyCode::Char('e'));
    h.press(KeyCode::Enter);
    for _ in 0..4 {
        h.press(KeyCode::Tab);
    }
    h.type_str("first-password");
    h.press(KeyCode::Enter);

    assert!(
        h.app.vault_creating(),
        "saving the first password must offer a vault"
    );
    h.type_str("master");
    h.draw();
    h.press(KeyCode::Esc);
    assert!(!h.app.vault_creating(), "Esc must decline the offer");
}

/// A terminal too small for the panel must clip, not panic.
#[tokio::test]
async fn the_panel_renders_in_a_terminal_too_small_for_it() {
    let _guard = setup().await;
    let mut h = Harness::new(&["host-a", "host-b"]);
    h.press(KeyCode::Char('e'));
    h.press(KeyCode::Enter);
    h.type_str("secret");

    for (w, hgt) in [(20u16, 5u16), (1, 1), (200, 60), (80, 2)] {
        h.terminal = Terminal::new(TestBackend::new(w, hgt)).unwrap();
        h.draw();
    }
}

/// Removing a server must leave the shorter list renderable and editable.
#[tokio::test]
async fn removing_a_server_leaves_a_renderable_panel() {
    let _guard = setup().await;
    let mut h = Harness::new(&["host-a", "host-b"]);

    h.press(KeyCode::Char('e'));
    h.press(KeyCode::Down);
    h.press(KeyCode::Char('d'));
    h.press(KeyCode::Char('y'));

    assert_eq!(h.app.panels.len(), 1, "{}", h.notice());
    h.press(KeyCode::Enter);
}

/// The Experimental block is where sparklines live, and `s` toggles them.
#[tokio::test]
async fn s_toggles_sparklines_from_the_experimental_block() {
    let _guard = setup().await;
    let mut h = Harness::new(&["host-a"]);
    h.press(KeyCode::Char('e'));

    let screen = h.screen();
    assert!(screen.contains("Experimental"), "got:\n{screen}");
    let before = h.app.show_sparklines();
    h.press(KeyCode::Char('s'));
    assert_ne!(h.app.show_sparklines(), before, "s must toggle sparklines");
}

// ---------------------------------------------------------------------------
// The structural gate: every short key sequence, drawn.
// ---------------------------------------------------------------------------

/// Keys the Configuration panel binds, plus the ones that edit text.
const SWEEP_KEYS: &[KeyCode] = &[
    KeyCode::Char('a'),
    KeyCode::Char('d'),
    KeyCode::Char('i'),
    KeyCode::Char('r'),
    KeyCode::Char('s'),
    KeyCode::Char('y'),
    KeyCode::Char('n'),
    KeyCode::Char('x'),
    KeyCode::Tab,
    KeyCode::Enter,
    KeyCode::Esc,
    KeyCode::Up,
    KeyCode::Down,
    KeyCode::Backspace,
];

/// How many presses deep the sweep goes.
///
/// Depth 4 has been run by hand and is clean, but takes minutes -- too slow to
/// sit in front of every commit. Raise it here when hunting, not permanently.
const DEPTH: usize = 3;

/// Every sequence of `DEPTH` presses from the open Configuration panel must
/// produce a frame that renders.
///
/// The class is "a state the panel can reach that the renderer cannot draw",
/// and only walking the reachable states rules it out.
#[tokio::test]
async fn every_short_key_sequence_in_the_panel_renders() {
    let _guard = setup().await;

    let mut sequence = [0usize; DEPTH];
    let total = SWEEP_KEYS.len().pow(u32::try_from(DEPTH).unwrap());
    for n in 0..total {
        let mut rest = n;
        for slot in &mut sequence {
            *slot = rest % SWEEP_KEYS.len();
            rest /= SWEEP_KEYS.len();
        }

        let mut h = Harness::new(&["host-a", "host-b"]);
        h.press(KeyCode::Char('e'));
        for &index in &sequence {
            h.press(SWEEP_KEYS[index]);
            // Answering the vault-creation prompt runs Argon2id sized to a
            // quarter of system RAM. It is covered on its own above; here it
            // would turn a sweep into an out-of-memory hazard, so the offer is
            // declined the moment it appears. Declining is a real key path.
            if h.app.vault_creating() {
                h.app.cancel_vault_creation();
                h.draw();
            }
        }
    }
}
