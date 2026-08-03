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
    let _ = password_store::delete_sso();
    guard
}

// ---------------------------------------------------------------------------
// The reported flow: enter a password in Server Settings.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn entering_an_sso_password_renders_at_every_keystroke() {
    let _guard = setup().await;
    let mut h = Harness::new(&["host-a", "host-b"]);

    h.press(KeyCode::Char('e'));
    assert!(h.app.password_manager.is_some(), "the panel must open");
    h.press(KeyCode::Char('s'));
    assert!(h.screen().contains("Password:"), "the prompt must be drawn");

    h.type_str("hunter2");
    assert!(
        h.screen().contains("*******"),
        "the typed password must be masked on screen, got:\n{}",
        h.screen()
    );

    h.press(KeyCode::Enter);
    assert_eq!(
        password_store::load_sso().unwrap().as_deref(),
        Some("hunter2"),
        "the password must reach the credential store"
    );
}

#[tokio::test]
async fn entering_a_per_host_override_renders_at_every_keystroke() {
    let _guard = setup().await;
    let mut h = Harness::new(&["host-a", "host-b"]);

    h.press(KeyCode::Char('e'));
    h.press(KeyCode::Down);
    h.press(KeyCode::Char('o'));
    assert!(
        h.screen().contains("host-b"),
        "the override prompt must name the host it applies to, got:\n{}",
        h.screen()
    );

    h.type_str("per-host");
    h.press(KeyCode::Enter);

    assert_eq!(
        password_store::load(&server("host-b")).unwrap().as_deref(),
        Some("per-host")
    );
}

/// The Servers section carries a password field inside the draft.
#[tokio::test]
async fn entering_a_password_in_the_server_draft_renders_at_every_keystroke() {
    let _guard = setup().await;
    let mut h = Harness::new(&["host-a"]);

    h.press(KeyCode::Char('e'));
    h.press(KeyCode::Tab); // to Servers
    h.press(KeyCode::Char('a')); // new draft
    h.type_str("host-new");
    h.press(KeyCode::Tab);
    h.type_str("admin");
    h.press(KeyCode::Tab);
    h.type_str("22");
    h.press(KeyCode::Tab);
    h.type_str("ls -l");
    h.press(KeyCode::Tab); // password field
    h.type_str("draft-secret");
    assert!(
        h.screen().contains("Sudo password: ************"),
        "the draft password must be masked, got:\n{}",
        h.screen()
    );

    h.press(KeyCode::Enter);
    assert_eq!(h.app.panels.len(), 2, "the server must be added");
}

/// Setting one SSO password flipped every host to "Stored", which reads as
/// "configured and working". Nothing has checked that password against any of
/// them, and when sudo later refused it the failure surfaced as a broken
/// upgrade command. The panel has to say which credential a host is using.
#[tokio::test]
async fn an_sso_password_is_labelled_as_borrowed_not_verified() {
    let _guard = setup().await;
    let mut h = Harness::new(&["host-a", "host-b"]);

    h.press(KeyCode::Char('e'));
    h.press(KeyCode::Char('s'));
    h.type_str("one-password-for-all");
    h.press(KeyCode::Enter);

    let screen = h.screen();
    assert!(
        screen.contains("SSO (unverified)"),
        "a borrowed password must not be shown as this host's own, got:\n{screen}"
    );
    assert!(
        !screen.contains("\u{2713} Stored"),
        "nothing here was verified against a host, got:\n{screen}"
    );
}

/// A password entered for one host specifically is that host's own.
#[tokio::test]
async fn a_per_host_password_supersedes_the_sso_label() {
    let _guard = setup().await;
    let mut h = Harness::new(&["host-a", "host-b"]);

    h.press(KeyCode::Char('e'));
    h.press(KeyCode::Char('s'));
    h.type_str("shared");
    h.press(KeyCode::Enter);
    // Now give host-a its own. Saving the first per-host password offers to
    // create a vault, which closes the panel -- decline and reopen.
    h.press(KeyCode::Char('o'));
    h.type_str("just-for-a");
    h.press(KeyCode::Enter);
    if h.app.vault_creating() {
        h.press(KeyCode::Esc);
    }
    if h.app.password_manager.is_none() {
        h.press(KeyCode::Char('e'));
    }

    let screen = h.screen();
    assert!(
        screen.contains("\u{2713} Stored"),
        "host-a now has its own password, got:\n{screen}"
    );
    assert!(
        screen.contains("SSO (unverified)"),
        "host-b is still borrowing, got:\n{screen}"
    );
}

/// Non-ASCII input must survive the round trip through the mask and the buffer.
#[tokio::test]
async fn a_password_with_wide_and_multibyte_characters_renders() {
    let _guard = setup().await;
    let mut h = Harness::new(&["host-a"]);

    h.press(KeyCode::Char('e'));
    h.press(KeyCode::Char('s'));
    h.type_str("pä55wörd-\u{4f60}\u{597d}");
    h.press(KeyCode::Backspace);
    h.press(KeyCode::Enter);

    assert_eq!(
        password_store::load_sso().unwrap().as_deref(),
        Some("pä55wörd-\u{4f60}")
    );
}

/// A terminal too small for the panel must clip, not panic.
#[tokio::test]
async fn the_panel_renders_in_a_terminal_too_small_for_it() {
    let _guard = setup().await;
    let mut h = Harness::new(&["host-a", "host-b"]);
    h.press(KeyCode::Char('e'));
    h.press(KeyCode::Char('s'));
    h.type_str("secret");

    for (w, hgt) in [(20u16, 5u16), (1, 1), (200, 60), (80, 2)] {
        h.terminal = Terminal::new(TestBackend::new(w, hgt)).unwrap();
        h.draw();
    }
}

// ---------------------------------------------------------------------------
// The structural gate: every short key sequence, drawn.
// ---------------------------------------------------------------------------

/// Keys the Configuration panel binds, plus the ones that edit text.
const SWEEP_KEYS: &[KeyCode] = &[
    KeyCode::Char('a'),
    KeyCode::Char('d'),
    KeyCode::Char('i'),
    KeyCode::Char('o'),
    KeyCode::Char('p'),
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
/// Depth 4 (65536 sequences) has been run by hand and is clean, but takes about
/// three minutes -- too slow to sit in front of every commit. Raise it here when
/// hunting, not permanently.
const DEPTH: usize = 3;

/// Every sequence of `DEPTH` presses from the open Configuration panel must
/// produce a frame that renders.
///
/// This is the part that generalises. The crash that prompted these tests was
/// one path through the panel; the class is "a state the panel can reach that
/// the renderer cannot draw", and only walking the reachable states rules that
/// class out. Depth 3 covers every binding plus its two-key follow-ups
/// (`d`-then-`y`, `s`-then-text-then-Enter, `a`-then-Tab).
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
            // quarter of system RAM. It is covered on its own below; here it
            // would turn a sweep into an out-of-memory hazard, so the offer is
            // declined the moment it appears. Declining is a real key path.
            if h.app.vault_creating() {
                h.app.cancel_vault_creation();
                h.draw();
            }
        }
    }
}

/// The vault-creation prompt is offered right after the first password is
/// saved, so it is part of "entering a password" and has to draw.
#[tokio::test]
async fn the_vault_creation_offer_renders_and_can_be_declined() {
    let _guard = setup().await;
    let mut h = Harness::new(&["host-a"]);

    h.press(KeyCode::Char('e'));
    h.press(KeyCode::Char('o'));
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

/// Removing a server mid-session shrinks the panel list; the panel must still
/// draw against the shorter list on the very next frame.
#[tokio::test]
async fn removing_a_server_leaves_a_renderable_panel() {
    let _guard = setup().await;
    let mut h = Harness::new(&["host-a", "host-b"]);

    h.press(KeyCode::Char('e'));
    h.press(KeyCode::Tab);
    h.press(KeyCode::Down); // select host-b, the last row
    h.press(KeyCode::Char('d'));
    h.press(KeyCode::Char('y'));

    assert_eq!(h.app.panels.len(), 1, "{}", h.notice());
    h.press(KeyCode::Tab); // back to Passwords, which indexes by `selected`
    h.press(KeyCode::Char('o'));
}
