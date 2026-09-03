//! Live end-to-end tests for the update view against a REAL host over SSH.
//!
//! Every bug in this area was found by a person running the app, not by the
//! suite, because the unit tests feed hand-written messages into `App`. The
//! failures all lived in the seam between the task that produces messages and
//! the state that consumes them: `AuxBegin` overwriting the pane, `Status`
//! overwriting the pane, a spawn failure never reporting `AuxDone`. Hand-written
//! messages cannot catch that, because writing them means deciding which
//! messages get sent -- the exact thing that was wrong.
//!
//! So these drive the real path end to end: real key presses through
//! `run::handle_key`, which really spawns `spawn_upgrade`, which really runs a
//! command over SSH, whose real messages are pumped into a real `App`, which is
//! really rendered. Nothing about the message sequence is assumed.
//!
//! # Never run a real upgrade command here
//!
//! Same rule as `upgrade_loop_remote_e2e.rs`: the upgrade command is a
//! read-only stand-in (`ls -l ; ls -l`) so the full SSH, streaming, sudo and
//! exit-code paths are exercised without touching packages on the target. The
//! servers are built from environment variables only and `config.toml` is never
//! read, so a real `upgrade_cmd` cannot leak in.
//!
//! Run:
//! ```
//! MULTITOP_TEST_SSH_HOST=<host> MULTITOP_TEST_SSH_USER=<user> \
//!   cargo test --test upgrade_view_live_e2e -- --ignored --test-threads=1
//! ```

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::sync::Arc;
use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
use ratatui::{backend::TestBackend, Terminal};
use tokio::sync::{mpsc, watch};

use multitop::app::{App, Mode, Msg};
use multitop::config::Server;
use multitop::panel::UpgradeState;
use multitop::run::{handle_key, Tasks};

/// Divert credentials to the in-memory store, and hold the process-global guard.
///
/// Driving an `App` reaches `password_store` several calls down, and an
/// integration binary is compiled without `cfg(test)`, so the mock is not in
/// force unless it is asked for. Without this these tests query the real OS
/// keychain: every rebuild changes the binary's code signature, so macOS raises
/// an access dialog and the suite stops until a human dismisses it -- and a test
/// can read, overwrite or delete credentials the user depends on.
#[allow(dead_code)]
fn isolate_keychain() -> tokio::sync::MutexGuard<'static, ()> {
    let guard = multitop::password_store::lock_for_test();
    multitop::password_store::enable_mock_store();
    multitop::password_store::clear_mock_store();
    guard
}

/// `isolate_keychain` for `#[tokio::test]` bodies, which must not block the
/// runtime thread to take the guard.
#[allow(dead_code)]
async fn isolate_keychain_async() -> tokio::sync::MutexGuard<'static, ()> {
    let guard = multitop::password_store::lock_for_test_async().await;
    multitop::password_store::enable_mock_store();
    multitop::password_store::clear_mock_store();
    guard
}

/// A read-only stand-in for a real upgrade. See the module header.
const SAFE_CMD: &str = "ls -l ; ls -l";

fn ssh_server(cmd: &str) -> Server {
    Server {
        host: std::env::var("MULTITOP_TEST_SSH_HOST").unwrap_or_else(|_| "127.0.0.1".into()),
        user: std::env::var("MULTITOP_TEST_SSH_USER")
            .unwrap_or_else(|_| std::env::var("USER").unwrap_or_else(|_| "root".into())),
        port: std::env::var("MULTITOP_TEST_SSH_PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(22),
        upgrade_cmd: Some(cmd.to_string()),
        custom_command: None,
    }
}

struct Live {
    app: App,
    tasks: Tasks,
    tx: mpsc::Sender<Msg>,
    rx: mpsc::Receiver<Msg>,
    dims_rx: Arc<watch::Receiver<(u16, u16)>>,
}

impl Live {
    fn new(servers: Vec<Server>) -> Self {
        let (tx, rx) = mpsc::channel::<Msg>(512);
        let (dims_tx, drx) = watch::channel((100u16, 30u16));
        drop(dims_tx);
        let panels = servers.len();
        Self {
            app: App::new(servers),
            tasks: Tasks::new(panels),
            tx,
            rx,
            dims_rx: Arc::new(drx),
        }
    }

    /// A real key press, through the real handler, which really spawns tasks.
    fn press(&mut self, c: char) {
        handle_key(
            KeyEvent {
                code: KeyCode::Char(c),
                modifiers: KeyModifiers::NONE,
                kind: KeyEventKind::Press,
                state: KeyEventState::NONE,
            },
            &mut self.app,
            (100, 30),
            Arc::clone(&self.dims_rx),
            &self.tx,
            &mut self.tasks,
        );
    }

    /// Apply whatever the live task has produced so far, without blocking.
    fn pump(&mut self) -> usize {
        let mut n = 0;
        while let Ok(msg) = self.rx.try_recv() {
            self.app.apply(msg);
            n += 1;
        }
        n
    }

    /// Pump until every upgrade reaches a terminal state, or time out.
    ///
    /// A timeout here is itself a finding: it means some exit path failed to
    /// report `AuxDone`, which is what leaves a panel stuck on "running".
    async fn pump_until_done(&mut self, limit: Duration) -> bool {
        let deadline = tokio::time::Instant::now() + limit;
        while tokio::time::Instant::now() < deadline {
            match tokio::time::timeout(Duration::from_millis(250), self.rx.recv()).await {
                Ok(Some(msg)) => {
                    self.app.apply(msg);
                }
                Ok(None) => break,
                Err(_) => {}
            }
            if !self.app.upgrades_in_flight() {
                self.pump();
                return true;
            }
        }
        false
    }

    fn pane(&self, panel: usize) -> String {
        strip_ansi(&self.app.panels[panel].view.join("\n"))
    }

    /// What the user would actually see, after layout and truncation.
    fn rendered(&mut self) -> String {
        let backend = TestBackend::new(100, 30);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| multitop::ui::draw(f, &mut self.app)).unwrap();
        let buf = term.backend().buffer().clone();
        (0..buf.area.height)
            .map(|y| {
                (0..buf.area.width)
                    .map(|x| buf.cell((x, y)).map_or(" ", ratatui::buffer::Cell::symbol))
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n")
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

/// The whole pane, against a real run: the status block must survive every
/// message the live task actually sends, and the output must arrive under it.
///
/// This is the test that would have caught `AuxBegin` and `Status` overwriting
/// the view, because it never decides what those messages are -- the real task
/// sends them.
#[ignore = "requires a reachable SSH host (MULTITOP_TEST_SSH_HOST); run with --ignored"]
#[tokio::test]
async fn live_run_keeps_its_status_block_and_collects_output() {
    let _keychain = isolate_keychain_async().await;
    let mut h = Live::new(vec![ssh_server(SAFE_CMD)]);

    h.press('u');
    assert_eq!(h.app.panels[0].mode, Mode::Upgrade);
    h.press('u');
    h.press('u');
    assert!(h.app.upgrades_in_flight(), "the run must actually start");

    assert!(
        h.pump_until_done(Duration::from_secs(60)).await,
        "the upgrade never reported completion -- a panel is stuck on running"
    );

    let pane = h.pane(0);
    assert!(
        pane.contains(SAFE_CMD),
        "the status block must survive a real run: {pane}"
    );
    assert!(
        pane.contains("Last run"),
        "the status block must be intact: {pane}"
    );
    assert!(
        pane.contains("total ") || pane.contains("drwx") || pane.contains("-rw"),
        "the command's real output must be in the pane: {pane}"
    );

    // And it survives layout, which is where the header used to be overwritten
    // by the panel banner.
    let screen = strip_ansi(&h.rendered());
    assert!(
        screen.contains("Command"),
        "the header must be visible on screen: {screen}"
    );
}

/// The reported case: leave the pane mid-run and come back. The run continues,
/// the output produced while away is not lost, and the completion marker is
/// there on return.
#[ignore = "requires a reachable SSH host (MULTITOP_TEST_SSH_HOST); run with --ignored"]
#[tokio::test]
async fn switching_views_during_a_live_run_loses_nothing() {
    let _keychain = isolate_keychain_async().await;
    // Long enough that we are reliably still running when we switch away.
    let mut h = Live::new(vec![ssh_server(
        "ls -l ; sleep 3 ; ls -l ; echo TAIL_MARKER",
    )]);

    h.press('u');
    h.press('u');
    h.press('u');
    assert!(h.app.upgrades_in_flight());

    // Let some output land while we are watching.
    tokio::time::sleep(Duration::from_millis(400)).await;
    h.pump();

    // Leave for the stats view while it is still running.
    h.press('s');
    assert_eq!(h.app.panels[0].mode, Mode::Monitor);
    assert!(
        h.app.upgrades_in_flight(),
        "leaving the view must not stop the run"
    );

    // Everything that arrives now arrives while the user is elsewhere.
    assert!(
        h.pump_until_done(Duration::from_secs(60)).await,
        "the run must finish in the background while away"
    );
    let stats = h.pane(0);
    assert!(
        !stats.contains("TAIL_MARKER"),
        "upgrade output must not leak into the stats view: {stats}"
    );

    // Come back.
    h.press('u');
    assert_eq!(h.app.panels[0].mode, Mode::Upgrade);
    let pane = h.pane(0);
    assert!(
        pane.contains("TAIL_MARKER"),
        "output produced while away must be there on return: {pane}"
    );
    assert!(
        pane.contains("done"),
        "and so must the completion marker: {pane}"
    );
    assert!(
        !pane.contains("do not quit"),
        "a finished run must stop claiming to be running: {pane}"
    );
}

/// A command that fails on a perfectly reachable host must say so, reach a
/// terminal state, and leave the app able to try again. Reporting this as a
/// disconnect sent the user looking at the network for a script that exited 2.
#[ignore = "requires a reachable SSH host (MULTITOP_TEST_SSH_HOST); run with --ignored"]
#[tokio::test]
async fn a_failing_command_on_a_reachable_host_is_reported_honestly() {
    let _keychain = isolate_keychain_async().await;
    let mut h = Live::new(vec![ssh_server("ls -l ; exit 2")]);

    h.press('u');
    h.press('u');
    h.press('u');
    assert!(
        h.pump_until_done(Duration::from_secs(60)).await,
        "a failing command must still reach a terminal state"
    );

    assert_eq!(h.app.panels[0].upgrade_state, UpgradeState::DONE);
    let pane = h.pane(0);
    assert!(
        pane.contains("exited 2"),
        "the exit code must be reported: {pane}"
    );
    assert!(
        !pane.contains("disconnected"),
        "a reachable host must not be reported as disconnected: {pane}"
    );
    assert!(
        pane.contains("last run failed"),
        "the badge must show the failure: {pane}"
    );

    // The failure must not wedge the app.
    assert!(!h.app.upgrades_in_flight());
    h.press('u');
    assert!(
        h.app.show_upgrade_modal(),
        "the user must be able to try again after a failure"
    );
}

/// An unreachable host must not leave the panel stuck on "running" forever.
/// This one needs no live host: the point is the spawn failure path.
#[ignore = "requires a reachable SSH host (MULTITOP_TEST_SSH_HOST); run with --ignored"]
#[tokio::test]
async fn an_unreachable_host_reaches_a_terminal_state() {
    let _keychain = isolate_keychain_async().await;
    let mut h = Live::new(vec![Server {
        // Reserved for documentation; never routable.
        host: "192.0.2.1".into(),
        port: 22,
        user: "nobody".into(),
        upgrade_cmd: Some(SAFE_CMD.to_string()),
        custom_command: None,
    }]);

    h.press('u');
    h.press('u');
    h.press('u');

    assert!(
        h.pump_until_done(Duration::from_secs(90)).await,
        "an unreachable host must still report completion, or the panel is \
         stuck on running and blocks every later upgrade"
    );
    assert!(!h.app.upgrades_in_flight());
    let pane = h.pane(0);
    assert!(
        pane.contains("Command"),
        "the status block must survive the failure: {pane}"
    );
}
