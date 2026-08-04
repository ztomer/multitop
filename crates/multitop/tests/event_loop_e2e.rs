//! The event loop, driven end to end against a test backend.
//!
//! Everything in `run::event_loop` used to be unreachable from a test: it took
//! the real terminal and read the real stdin, so the only way to exercise it
//! was a person watching a real terminal. Every defect it has ever shipped was
//! found that way. This drives the loop with an injected backend and an
//! injected event stream instead.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::atomic::{AtomicU16, Ordering};
use std::time::Duration;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use multitop::config::Server;
use multitop::password_store;
use ratatui::backend::TestBackend;
use tokio_stream::StreamExt as _;

static PORT_COUNTER: AtomicU16 = AtomicU16::new(41000);

/// Callers pass `.example` hosts, a name RFC 2606 reserves and no resolver
/// answers, so a monitor task that reaches for one during the test cannot
/// touch anything real.
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

/// Divert credentials to the in-memory store. An integration binary is not
/// compiled with `cfg(test)`, so without this the server edit below reaches the
/// real OS keychain and the suite stops on a dialog.
async fn isolate_keychain() -> tokio::sync::MutexGuard<'static, ()> {
    let guard = password_store::lock_for_test_async().await;
    password_store::enable_mock_store();
    password_store::clear_mock_store();
    guard
}

struct Harness {
    _dir: tempfile::TempDir,
    dims: tokio::sync::watch::Receiver<(u16, u16)>,
    loop_task: tokio::task::JoinHandle<multitop::run::LoopOutcome>,
}

impl Harness {
    /// Start the loop on a background task with `events` scripted, then left
    /// open so the loop keeps running until the test aborts it.
    fn start(servers: Vec<Server>, size: (u16, u16), events: Vec<Event>) -> Self {
        let dir = tempfile::tempdir().expect("temp dir");
        let config_path = dir.path().join("config.toml");
        let (dims_tx, dims_rx) = tokio::sync::watch::channel((0, 0));
        let mut stream =
            tokio_stream::iter(events.into_iter().map(Ok)).chain(tokio_stream::pending());
        let loop_task = tokio::spawn(async move {
            let backend = TestBackend::new(size.0, size.1);
            let mut terminal = ratatui::Terminal::new(backend).expect("terminal");
            multitop::run::event_loop(
                &mut terminal,
                &mut stream,
                dims_tx,
                servers,
                config_path,
                None,
            )
            .await
        });
        Self {
            _dir: dir,
            dims: dims_rx,
            loop_task,
        }
    }

    /// Wait until the published agent render size settles on `want`, or fail
    /// saying what it settled on instead.
    ///
    /// The watch channel keeps only the newest value, so this waits for the
    /// value to *arrive* rather than asserting on a snapshot -- the loop
    /// processes a scripted burst faster than the test can look at it.
    async fn expect_dims(&mut self, want: (u16, u16), what: &str) {
        let waited = tokio::time::timeout(Duration::from_secs(5), async {
            while *self.dims.borrow_and_update() != want {
                if self.dims.changed().await.is_err() {
                    return;
                }
            }
        })
        .await;
        let got = *self.dims.borrow();
        assert!(
            waited.is_ok() && got == want,
            "{what}: the agents were told to render at {got:?}, expected {want:?}"
        );
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.loop_task.abort();
    }
}

fn dims_for(size: (u16, u16), panels: usize) -> (u16, u16) {
    multitop::ui::agent_dims(
        ratatui::layout::Size {
            width: size.0,
            height: size.1,
        },
        panels,
    )
}

/// Removing a server changes the grid -- three panels are two columns, two are
/// one -- so it changes the size every pane gets. The agents render into that
/// size, and they have to be told.
///
/// This failed before the fix: the render size was recomputed only when a
/// `Resize` arrived, so an edit to the server list left every agent drawing for
/// the old grid until the user happened to resize the window.
#[tokio::test]
async fn removing_a_server_resizes_what_the_agents_render() {
    let _keychain = isolate_keychain().await;
    let size = (100, 30);
    let servers = vec![
        test_server("alpha.example"),
        test_server("beta.example"),
        test_server("gamma.example"),
    ];

    let mut h = Harness::start(
        servers,
        size,
        vec![
            // Settings, then remove the selected host: `d` asks, `y` answers.
            key(KeyCode::Char('e')),
            key(KeyCode::Char('d')),
            key(KeyCode::Char('y')),
        ],
    );

    // Not asserted on the way through: the loop consumes the scripted burst
    // faster than the test can observe an intermediate value, and the watch
    // channel keeps only the newest. What matters is where it lands.
    assert_ne!(
        dims_for(size, 3),
        dims_for(size, 2),
        "this test is only meaningful while the panel count changes the size"
    );
    h.expect_dims(dims_for(size, 2), "after the removal").await;
}

/// The render size is derived from the terminal size *and* the panel count.
///
/// The count used to be captured before the first frame and never updated, so
/// the next resize after a server edit recomputed the size from the old count
/// and put the wrong value back -- the one case where resizing the window made
/// the display worse.
#[tokio::test]
async fn a_resize_after_a_server_edit_uses_the_new_count() {
    let _keychain = isolate_keychain().await;
    let size = (100, 30);
    let servers = vec![
        test_server("alpha.example"),
        test_server("beta.example"),
        test_server("gamma.example"),
    ];

    let mut h = Harness::start(
        servers,
        size,
        vec![
            key(KeyCode::Char('e')),
            key(KeyCode::Char('d')),
            key(KeyCode::Char('y')),
            key(KeyCode::Esc),
            // Same terminal size: what is under test is the count, not the size.
            Event::Resize(size.0, size.1),
        ],
    );

    h.expect_dims(dims_for(size, 2), "after the removal").await;
    // Long enough for the resize debounce to fire and publish, if it is going
    // to publish anything at all.
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        *h.dims.borrow(),
        dims_for(size, 2),
        "the resize recomputed the render size from the panel count the app \
         started with, not the one it has"
    );
}

/// A terminal that fails mid-frame still has to say which upgrades it killed.
/// The notice used to sit behind a `?` on the loop's result.
#[test]
fn the_outcome_carries_both_the_error_and_the_killed_hosts() {
    // A compile-level guard: `LoopOutcome` must keep both fields reachable, so
    // the caller cannot go back to a `Result` that can only carry one.
    let outcome = multitop::run::LoopOutcome {
        killed: vec!["db-02".to_string()],
        error: Some(std::io::Error::other("terminal went away")),
    };
    assert_eq!(outcome.killed, vec!["db-02".to_string()]);
    assert!(outcome.error.is_some());
}
