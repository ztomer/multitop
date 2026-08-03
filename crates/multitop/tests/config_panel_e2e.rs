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

    /// Deliver every message the spawned work has produced, exactly as the
    /// event loop does, and return how many there were.
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
            self.app.apply(msg);
            delivered += 1;
        }
        self.draw();
        delivered
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

/// Walk from the row editor to a created vault, the way a user does.
///
/// Returns with the vault made and every message delivered.
async fn create_a_vault(h: &mut Harness, master: &str) -> usize {
    h.press(KeyCode::Char('e'));
    h.press(KeyCode::Enter);
    for _ in 0..4 {
        h.press(KeyCode::Tab);
    }
    h.type_str("host-secret");
    h.press(KeyCode::Enter);
    assert!(h.app.vault_creating(), "the first password offers a vault");
    h.type_str(master);
    h.press(KeyCode::Enter);
    h.pump(std::time::Duration::from_secs(10)).await
}

/// Answering the vault offer must leave the user where they were.
///
/// Reported: "when setting the vault password, stay on the settings pane, do
/// not switch back to the stats panel." It did switch, because the renderer
/// drew either the configuration panel or a modal and never both, so the offer
/// could only be shown by closing the panel first.
#[tokio::test]
async fn creating_a_vault_from_server_settings_stays_in_server_settings() {
    let _guard = setup().await;
    let mut h = Harness::new(&["host-a"]);

    h.press(KeyCode::Char('e'));
    h.press(KeyCode::Enter);
    for _ in 0..4 {
        h.press(KeyCode::Tab);
    }
    h.type_str("host-secret");
    h.press(KeyCode::Enter);

    assert!(h.app.vault_creating(), "the offer must appear");
    assert!(
        h.app.password_manager.is_some(),
        "the offer must not close Server Settings"
    );
    assert!(
        h.screen().contains("Create Vault"),
        "and the prompt must be drawn over the panel, got:\n{}",
        h.screen()
    );

    // Declining is the same story: back to the list, not to the stats screen.
    h.press(KeyCode::Esc);
    assert!(!h.app.vault_creating());
    assert!(
        h.app.password_manager.is_some(),
        "Esc returns to the settings list"
    );
    assert!(
        h.screen().contains("Server Settings"),
        "got:\n{}",
        h.screen()
    );
}

/// The master password is taken once, however many times Enter is pressed.
///
/// Reported: "I had to enter the vault password three times when creating it."
/// Enter handed the password to Argon2id and left the prompt on screen with an
/// empty field, so it read as not having taken -- and every re-submission
/// initialised the vault again. The later attempts failed (a vault existed by
/// then) and their failures, carrying the same epoch as the first attempt's
/// success, put the creation prompt back up over a working vault.
#[tokio::test]
async fn a_second_enter_cannot_start_a_second_vault_creation() {
    let _guard = setup().await;
    let mut h = Harness::new(&["host-a"]);

    h.press(KeyCode::Char('e'));
    h.press(KeyCode::Enter);
    for _ in 0..4 {
        h.press(KeyCode::Tab);
    }
    h.type_str("host-secret");
    h.press(KeyCode::Enter);
    assert!(h.app.vault_creating());

    h.type_str("master-once");
    h.press(KeyCode::Enter);
    assert!(
        h.app.vault_create_in_flight(),
        "the prompt must report that it took the password"
    );
    let screen = h.screen();
    assert!(
        screen.contains("Creating Vault"),
        "and say so on screen, got:\n{screen}"
    );

    // What a user does when a field looks empty: type it again. Twice.
    h.type_str("master-once");
    h.press(KeyCode::Enter);
    h.type_str("master-once");
    h.press(KeyCode::Enter);

    let delivered = h.pump(std::time::Duration::from_secs(10)).await;
    assert_eq!(
        delivered, 1,
        "one Enter, one attempt -- extra presses must not initialise the vault again"
    );
    assert!(
        !h.app.vault_creating(),
        "the prompt must be gone once the vault exists, not back with an error: {:?}",
        h.app.vault_create_error()
    );
    assert!(h.app.vault.is_some(), "and the vault must exist");
    assert!(
        h.app.password_manager.is_some(),
        "still in Server Settings afterwards"
    );
    assert!(
        h.notice().contains("Vault created"),
        "with the outcome said where the user is looking, got: {:?}",
        h.notice()
    );
}

/// The password that started all this must be in the vault it created.
#[tokio::test]
async fn the_password_that_offered_the_vault_is_stored_in_it() {
    let _guard = setup().await;
    let mut h = Harness::new(&["host-a"]);

    assert_eq!(create_a_vault(&mut h, "master-once").await, 1);
    assert!(h.app.vault.is_some());
    assert_eq!(
        h.app.panels[0].sudo_password.as_deref(),
        Some("host-secret")
    );
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
    KeyCode::Char('e'),
    KeyCode::Char('i'),
    KeyCode::Char('q'),
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
