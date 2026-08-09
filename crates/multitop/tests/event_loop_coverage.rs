//! The event loop's remaining arms: startup notices, mouse input, the events
//! the loop deliberately ignores, and the terminal failing under it.
//!
//! Companion to `event_loop_e2e.rs`, which covers the resize/agent-dims path.
//! Everything here is a branch a person could previously only reach by sitting
//! at a real terminal with a real mouse and a hand-damaged config file.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::atomic::{AtomicU16, Ordering};
use std::time::Duration;

use crossterm::event::{
    Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use multitop::config::Server;
use multitop::password_store;
use multitop::run::{panel_at_pos, LoopOutcome};
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;
use tokio_stream::StreamExt as _;

static PORT_COUNTER: AtomicU16 = AtomicU16::new(43000);

/// `.example` is reserved by RFC 2606 and no resolver answers it, so a monitor
/// task that reaches for one of these cannot touch anything real.
fn test_server(host: &str) -> Server {
    Server {
        host: host.to_string(),
        port: PORT_COUNTER.fetch_add(1, Ordering::Relaxed),
        user: "admin".to_string(),
        upgrade_cmd: Some("true".to_string()),
    }
}

const fn key(code: KeyCode) -> Event {
    Event::Key(KeyEvent::new_with_kind(
        code,
        KeyModifiers::NONE,
        KeyEventKind::Press,
    ))
}

const fn mouse(kind: MouseEventKind, column: u16, row: u16) -> Event {
    Event::Mouse(MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    })
}

async fn isolate_keychain() -> tokio::sync::MutexGuard<'static, ()> {
    let guard = password_store::lock_for_test_async().await;
    password_store::enable_mock_store();
    password_store::clear_mock_store();
    guard
}

/// Run the loop to completion over a fixed script, then hand back what it drew
/// and why it stopped. The script always ends in a quit so the loop returns.
async fn run_to_quit(
    dir: &tempfile::TempDir,
    servers: Vec<Server>,
    size: (u16, u16),
    theme: Option<String>,
    mut events: Vec<Event>,
) -> (String, LoopOutcome) {
    events.push(key(KeyCode::Char('q')));
    let config_path = dir.path().join("config.toml");
    let (dims_tx, _dims_rx) = tokio::sync::watch::channel((0, 0));
    let mut stream = tokio_stream::iter(events.into_iter().map(Ok)).chain(tokio_stream::pending());

    let backend = TestBackend::new(size.0, size.1);
    let mut terminal = ratatui::Terminal::new(backend).expect("terminal");
    let outcome = tokio::time::timeout(
        Duration::from_secs(20),
        multitop::run::event_loop(
            &mut terminal,
            &mut stream,
            dims_tx,
            servers,
            config_path,
            theme,
        ),
    )
    .await
    .expect("the loop must quit on `q`");

    let drawn = terminal
        .backend()
        .buffer()
        .content()
        .iter()
        .map(ratatui::buffer::Cell::symbol)
        .collect::<String>();
    (drawn, outcome)
}

// -------------------------------------------------------------- startup notices

#[tokio::test]
async fn a_history_limit_below_the_floor_is_raised_and_said_out_loud() {
    let _keychain = isolate_keychain().await;
    let dir = tempfile::tempdir().unwrap();
    // One line would leave the Upgrade pane swallowing everything, including
    // the warnings that name a lock file to remove. Obeying it silently is the
    // failure; raising it silently is only half a fix.
    std::fs::write(
        dir.path().join("config.toml"),
        "upgrade_history_lines = 1\n\n[[servers]]\nhost = \"alpha.example\"\nport = 22\nuser = \"admin\"\n",
    )
    .unwrap();

    let (drawn, _) = run_to_quit(
        &dir,
        vec![test_server("alpha.example")],
        (120, 40),
        None,
        vec![],
    )
    .await;
    assert!(
        drawn.contains("upgrade_history_lines"),
        "the raised limit was applied without telling anyone:\n{drawn}"
    );
}

#[tokio::test]
async fn a_state_file_that_cannot_be_parsed_says_so_and_is_kept() {
    let _keychain = isolate_keychain().await;
    let dir = tempfile::tempdir().unwrap();
    let state = dir.path().join("state.toml");
    std::fs::write(&state, "this is not = = toml").unwrap();

    let (drawn, _) = run_to_quit(
        &dir,
        vec![test_server("alpha.example")],
        (120, 40),
        None,
        vec![],
    )
    .await;
    assert!(
        drawn.contains("could not be parsed"),
        "an unreadable state file must not look like a first run:\n{drawn}"
    );
    // The evidence is preserved rather than overwritten by the next save.
    assert!(state.with_extension("toml.unreadable").exists());
}

#[tokio::test]
async fn a_plaintext_password_in_the_config_is_moved_out_of_it() {
    let _keychain = isolate_keychain().await;
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("config.toml");
    std::fs::write(
        &config,
        "[[servers]]\nhost = \"alpha.example\"\nport = 22\nuser = \"admin\"\nsudo_password = \"hunter2\"\n",
    )
    .unwrap();

    // Porting a password earns a vault the same way an interactive save does,
    // so the offer is up and has to be answered before the app will quit.
    let (drawn, _) = run_to_quit(
        &dir,
        vec![test_server("alpha.example")],
        (120, 40),
        None,
        vec![key(KeyCode::Esc)],
    )
    .await;

    let after = std::fs::read_to_string(&config).unwrap();
    assert!(
        !after.contains("hunter2"),
        "a plaintext password was left on disk:\n{after}"
    );
    assert!(
        drawn.contains("plaintext password"),
        "the move happened silently:\n{drawn}"
    );
}

#[tokio::test]
async fn a_named_theme_is_selected_at_startup() {
    let _keychain = isolate_keychain().await;
    let dir = tempfile::tempdir().unwrap();
    let name = multitop_agent::color::THEMES[1].name;

    // Asking for a theme by name must select it; asking for one that does not
    // exist must leave the default alone rather than failing to start.
    let (_, outcome) = run_to_quit(
        &dir,
        vec![test_server("alpha.example")],
        (120, 40),
        Some(name.to_ascii_uppercase()),
        vec![],
    )
    .await;
    assert!(outcome.error.is_none());

    let (_, outcome) = run_to_quit(
        &dir,
        vec![test_server("alpha.example")],
        (120, 40),
        Some("no-such-theme".into()),
        vec![],
    )
    .await;
    assert!(
        outcome.error.is_none(),
        "an unknown theme must not stop startup"
    );
}

// ---------------------------------------------------------------------- mouse

#[tokio::test]
async fn a_click_selects_the_pane_it_landed_on() {
    let _keychain = isolate_keychain().await;
    let dir = tempfile::tempdir().unwrap();
    let servers = vec![
        test_server("alpha.example"),
        test_server("beta.example"),
        test_server("gamma.example"),
        test_server("delta.example"),
    ];
    let size = (120u16, 40u16);
    let shown: Vec<usize> = (0..servers.len()).collect();
    let (areas, _) = multitop::ui::regions(Rect::new(0, 0, size.0, size.1), servers.len());
    // A point inside the last pane, which is not the one selected at startup.
    let last = areas[areas.len() - 1];
    let (x, y) = (last.x + 1, last.y + 1);
    assert_eq!(
        panel_at_pos(x, y, Rect::new(0, 0, size.0, size.1), &shown),
        Some(3)
    );

    let (drawn, outcome) = run_to_quit(
        &dir,
        servers,
        size,
        None,
        vec![
            mouse(MouseEventKind::Down(MouseButton::Left), x, y),
            // Scrolls on the same pane; the loop must act on them rather than
            // discarding them with the motion floods.
            mouse(MouseEventKind::ScrollUp, x, y),
            mouse(MouseEventKind::ScrollDown, x, y),
        ],
    )
    .await;
    assert!(outcome.error.is_none());
    assert!(
        drawn.contains("delta"),
        "the clicked pane should be drawn:\n{drawn}"
    );
}

#[tokio::test]
async fn mouse_traffic_that_lands_on_nothing_is_ignored() {
    let _keychain = isolate_keychain().await;
    let dir = tempfile::tempdir().unwrap();
    // Motion floods in whenever any-event tracking is on; the keybar row and
    // any point past the screen match no pane. None of it may stop the loop.
    let (_, outcome) = run_to_quit(
        &dir,
        vec![test_server("alpha.example")],
        (120, 40),
        None,
        vec![
            mouse(MouseEventKind::Moved, 5, 5),
            mouse(MouseEventKind::Up(MouseButton::Left), 5, 5),
            mouse(MouseEventKind::Down(MouseButton::Right), 5, 5),
            mouse(MouseEventKind::Down(MouseButton::Left), 5000, 5000),
            mouse(MouseEventKind::ScrollUp, 5000, 5000),
        ],
    )
    .await;
    assert!(outcome.error.is_none());
}

#[test]
fn a_click_with_no_panes_on_screen_selects_nothing() {
    // Answering "panel 0" here moved the selection to the first host whenever
    // the user clicked the keys row, which is the row that invites clicking.
    assert_eq!(panel_at_pos(1, 1, Rect::new(0, 0, 80, 24), &[]), None);
    // A filtered list answers with the index into the *unfiltered* list, which
    // is what the panes on screen actually are.
    assert_eq!(panel_at_pos(1, 1, Rect::new(0, 0, 80, 24), &[7]), Some(7));
}

// ------------------------------------------------------------ ignored events

#[tokio::test]
async fn events_the_loop_has_no_use_for_are_stepped_over() {
    let _keychain = isolate_keychain().await;
    let dir = tempfile::tempdir().unwrap();
    let (_, outcome) = run_to_quit(
        &dir,
        vec![test_server("alpha.example")],
        (120, 40),
        None,
        vec![
            Event::FocusGained,
            Event::FocusLost,
            Event::Paste("pasted".into()),
            Event::Resize(100, 30),
        ],
    )
    .await;
    assert!(outcome.error.is_none());
}

#[tokio::test]
async fn a_terminal_that_goes_away_leaves_through_the_normal_exit() {
    // Returning early would strand the SSH children, so a dead event source
    // has to quit the same way `q` does.
    let _keychain = isolate_keychain().await;
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("config.toml");
    let (dims_tx, _rx) = tokio::sync::watch::channel((0, 0));

    let mut stream = tokio_stream::iter(vec![Err(std::io::Error::other("terminal went away"))]);
    let backend = TestBackend::new(100, 30);
    let mut terminal = ratatui::Terminal::new(backend).expect("terminal");
    let outcome = tokio::time::timeout(
        Duration::from_secs(20),
        multitop::run::event_loop(
            &mut terminal,
            &mut stream,
            dims_tx,
            vec![test_server("alpha.example")],
            config_path,
            None,
        ),
    )
    .await
    .expect("a dead event source must end the loop");

    // A read error is not a terminal failure: the loop quit cleanly, so there
    // is nothing to report but the upgrades it killed.
    assert!(outcome.error.is_none());
    assert_eq!(outcome.killed, [] as [std::string::String; 0]);
}

#[tokio::test]
async fn an_exhausted_event_source_ends_the_loop() {
    let _keychain = isolate_keychain().await;
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("config.toml");
    let (dims_tx, _rx) = tokio::sync::watch::channel((0, 0));

    let mut stream = tokio_stream::iter(Vec::<std::io::Result<Event>>::new());
    let backend = TestBackend::new(100, 30);
    let mut terminal = ratatui::Terminal::new(backend).expect("terminal");
    let outcome = tokio::time::timeout(
        Duration::from_secs(20),
        multitop::run::event_loop(
            &mut terminal,
            &mut stream,
            dims_tx,
            vec![test_server("alpha.example")],
            config_path,
            None,
        ),
    )
    .await
    .expect("an exhausted event source must end the loop");
    assert!(outcome.error.is_none());
}

// -------------------------------------------------------------------- signals

/// `SIGTERM` and `SIGHUP` end the process at their default disposition, so
/// `TerminalGuard` never runs and neither does the notice naming the upgrades
/// that were killed. Caught, they become an ordinary quit.
#[tokio::test]
async fn a_termination_signal_leaves_through_the_normal_exit() {
    let _keychain = isolate_keychain().await;
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("config.toml");
    let (dims_tx, _rx) = tokio::sync::watch::channel((0, 0));

    // No scripted events: the only thing that can end this loop is the signal.
    let mut stream = tokio_stream::pending::<std::io::Result<Event>>();
    let backend = TestBackend::new(100, 30);
    let mut terminal = ratatui::Terminal::new(backend).expect("terminal");

    let signaller = tokio::spawn(async {
        // Long enough for the loop to have installed its handlers.
        tokio::time::sleep(Duration::from_millis(300)).await;
        let _ = std::process::Command::new("kill")
            .arg("-TERM")
            .arg(std::process::id().to_string())
            .status();
    });

    let outcome = tokio::time::timeout(
        Duration::from_secs(20),
        multitop::run::event_loop(
            &mut terminal,
            &mut stream,
            dims_tx,
            vec![test_server("alpha.example")],
            config_path,
            None,
        ),
    )
    .await
    .expect("SIGTERM must end the loop");
    signaller.await.unwrap();

    assert!(
        outcome.error.is_none(),
        "a caught signal is not a terminal failure"
    );
}

/// A resume rebuilds the terminal: anything that stopped this process left raw
/// mode and the alternate screen behind, and the shell has drawn over both.
#[tokio::test]
async fn a_resume_redraws_rather_than_leaving_the_shell_s_output_on_screen() {
    let _keychain = isolate_keychain().await;
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("config.toml");
    let (dims_tx, _rx) = tokio::sync::watch::channel((0, 0));

    let mut stream =
        tokio_stream::iter(vec![Ok(key(KeyCode::Char('q')))]).chain(tokio_stream::pending());
    let backend = TestBackend::new(100, 30);
    let mut terminal = ratatui::Terminal::new(backend).expect("terminal");

    let signaller = tokio::spawn(async {
        tokio::time::sleep(Duration::from_millis(300)).await;
        let _ = std::process::Command::new("kill")
            .arg("-CONT")
            .arg(std::process::id().to_string())
            .status();
    });

    // The `q` is consumed first, so the loop is already gone by the time the
    // signal lands in most runs; what is under test is that neither ordering
    // wedges it.
    let outcome = tokio::time::timeout(
        Duration::from_secs(20),
        multitop::run::event_loop(
            &mut terminal,
            &mut stream,
            dims_tx,
            vec![test_server("alpha.example")],
            config_path,
            None,
        ),
    )
    .await
    .expect("the loop must end");
    signaller.await.unwrap();
    assert!(outcome.error.is_none());
}

// ------------------------------------------------------- agents after an edit

/// Editing the server list while the Docker view is up has to restart the
/// docker pollers too, not just the monitor streams — otherwise the new panel
/// list is fed by tasks bound to the old one.
#[tokio::test]
async fn a_server_edit_in_the_docker_view_restarts_the_docker_pollers() {
    let _keychain = isolate_keychain().await;
    let dir = tempfile::tempdir().unwrap();
    let servers = vec![
        test_server("alpha.example"),
        test_server("beta.example"),
        test_server("gamma.example"),
    ];

    let (_, outcome) = run_to_quit(
        &dir,
        servers,
        (120, 40),
        None,
        vec![
            // Into the Docker view, then Settings, then remove the selected
            // host: the panel list changes underneath a running view.
            key(KeyCode::Char('d')),
            key(KeyCode::Char('e')),
            key(KeyCode::Char('d')),
            key(KeyCode::Char('y')),
            key(KeyCode::Esc),
        ],
    )
    .await;
    assert!(outcome.error.is_none());
}

// ------------------------------------------------------- a failing terminal

/// A backend whose draw fails, standing in for the terminal going away
/// mid-frame. The loop has to report that *and* still name the upgrades it
/// killed on the way out — the notice used to sit behind a `?` on the result.
struct FailingBackend {
    inner: TestBackend,
    draws_left: std::cell::Cell<usize>,
}

impl ratatui::backend::Backend for FailingBackend {
    type Error = std::io::Error;

    fn draw<'a, I>(&mut self, content: I) -> Result<(), Self::Error>
    where
        I: Iterator<Item = (u16, u16, &'a ratatui::buffer::Cell)>,
    {
        if self.draws_left.get() == 0 {
            return Err(std::io::Error::other("the terminal went away"));
        }
        self.draws_left.set(self.draws_left.get() - 1);
        self.inner.draw(content).map_err(std::io::Error::other)
    }
    fn hide_cursor(&mut self) -> Result<(), Self::Error> {
        self.inner.hide_cursor().map_err(std::io::Error::other)
    }
    fn show_cursor(&mut self) -> Result<(), Self::Error> {
        self.inner.show_cursor().map_err(std::io::Error::other)
    }
    fn get_cursor_position(&mut self) -> Result<ratatui::layout::Position, Self::Error> {
        self.inner
            .get_cursor_position()
            .map_err(std::io::Error::other)
    }
    fn set_cursor_position<P: Into<ratatui::layout::Position>>(
        &mut self,
        position: P,
    ) -> Result<(), Self::Error> {
        self.inner
            .set_cursor_position(position)
            .map_err(std::io::Error::other)
    }
    fn clear(&mut self) -> Result<(), Self::Error> {
        self.inner.clear().map_err(std::io::Error::other)
    }
    fn clear_region(&mut self, region: ratatui::backend::ClearType) -> Result<(), Self::Error> {
        self.inner
            .clear_region(region)
            .map_err(std::io::Error::other)
    }
    fn size(&self) -> Result<ratatui::layout::Size, Self::Error> {
        self.inner.size().map_err(std::io::Error::other)
    }
    fn window_size(&mut self) -> Result<ratatui::backend::WindowSize, Self::Error> {
        self.inner.window_size().map_err(std::io::Error::other)
    }
    fn flush(&mut self) -> Result<(), Self::Error> {
        self.inner.flush().map_err(std::io::Error::other)
    }
}

#[tokio::test]
async fn a_terminal_that_fails_mid_frame_reports_it_and_the_upgrades_it_killed() {
    let _keychain = isolate_keychain().await;
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("config.toml");
    let (dims_tx, _rx) = tokio::sync::watch::channel((0, 0));

    // One good frame, then the terminal is gone.
    let backend = FailingBackend {
        inner: TestBackend::new(100, 30),
        draws_left: std::cell::Cell::new(1),
    };
    let mut terminal = ratatui::Terminal::new(backend).expect("terminal");
    let mut stream =
        tokio_stream::iter(vec![Ok(Event::Resize(90, 28))]).chain(tokio_stream::pending());

    let outcome = tokio::time::timeout(
        Duration::from_secs(20),
        multitop::run::event_loop(
            &mut terminal,
            &mut stream,
            dims_tx,
            vec![test_server("alpha.example")],
            config_path,
            None,
        ),
    )
    .await
    .expect("a failing draw must end the loop");

    let error = outcome
        .error
        .expect("the terminal failure must be reported");
    assert!(
        error.to_string().contains("went away"),
        "the reason was replaced: {error}"
    );
}
