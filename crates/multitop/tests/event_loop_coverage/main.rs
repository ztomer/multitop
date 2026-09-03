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
        custom_command: None,
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
mod failing_backend;
mod signals_and_resume;
mod startup_and_input;
