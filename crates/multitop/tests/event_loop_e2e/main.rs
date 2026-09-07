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
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::StreamExt as _;

/// A panel whose upgrade runs as a real local command instead of reaching for
/// ssh: `127.0.0.1` is `is_local`, so `spawn_command` uses `$SHELL -c` and the
/// stream is real output, not a dying connection.
fn local_server(port: u16, cmd: &str) -> Server {
    Server {
        host: "127.0.0.1".to_string(),
        port,
        user: "admin".to_string(),
        upgrade_cmd: Some(cmd.to_string()),
        custom_command: None,
    }
}

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

/// Divert credentials to the in-memory store. An integration binary is not
/// compiled with `cfg(test)`, so without this the server edit below reaches the
/// real OS keychain and the suite stops on a dialog.
async fn isolate_keychain() -> tokio::sync::MutexGuard<'static, ()> {
    let guard = password_store::lock_for_test_async().await;
    password_store::set_mock_load_delay(None);
    password_store::enable_mock_store();
    password_store::clear_mock_store();
    guard
}

struct Harness {
    _dir: tempfile::TempDir,
    dims: tokio::sync::watch::Receiver<(u16, u16)>,
    loop_task: tokio::task::JoinHandle<multitop::run::LoopOutcome>,
}

/// Wait until the state file records `finished_at` for the host, then say
/// whether it did. Upgrades record their outcome through `AuxDone` on the loop
/// thread, so this polls the durable artifact rather than timing a guess.
fn state_says_finished(cfg: &std::path::Path, account: &str) -> bool {
    let path = multitop::state::state_file_path(cfg);
    let Ok(text) = std::fs::read_to_string(path) else {
        return false;
    };
    // The same parse the app's own loader trusts, so a record this says is
    // finished is a record the app believed too. `str::parse` is a *value*
    // parser here, not a document one, and rejects the file.
    let Ok(value) = toml::from_str::<toml::Value>(&text) else {
        return false;
    };
    value
        .get("hosts")
        .and_then(|h| h.get(account))
        .and_then(|entry| entry.get("finished_at"))
        .is_some_and(toml::Value::is_integer)
}

/// Whether the state file records `started_at` for the host -- the durable
/// mark of a run actually beginning. The confirm deferral must keep this absent
/// while a credential lookup is still in flight.
fn state_says_started(cfg: &std::path::Path, account: &str) -> bool {
    let path = multitop::state::state_file_path(cfg);
    let Ok(text) = std::fs::read_to_string(path) else {
        return false;
    };
    let Ok(value) = toml::from_str::<toml::Value>(&text) else {
        return false;
    };
    value
        .get("hosts")
        .and_then(|h| h.get(account))
        .and_then(|entry| entry.get("started_at"))
        .is_some_and(toml::Value::is_integer)
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
        // Ceiling, not a sleep: early when healthy. 5s tripped under
        // parallel-suite load before the loop had even started.
        let waited = tokio::time::timeout(Duration::from_secs(15), async {
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

/// The paced variant: an open event channel the test drives with real timing.
///
/// A key here is sent at the moment the test wants it to be *read* -- an
/// upgrade streaming through the same channel is the traffic it has to compete
/// with, which is the whole arrangement under test. Unlike `Harness`, exiting
/// is part of the contract, so `Drop` aborts the loop so a waiting test cannot
/// hang the suite.
struct PacedHarness {
    _dir: tempfile::TempDir,
    cfg: std::path::PathBuf,
    loop_task: Option<tokio::task::JoinHandle<multitop::run::LoopOutcome>>,
}

impl PacedHarness {
    fn start(
        servers: Vec<Server>,
        size: (u16, u16),
    ) -> (Self, tokio::sync::mpsc::Sender<std::io::Result<Event>>) {
        let dir = tempfile::tempdir().expect("temp dir");
        let config_path = dir.path().join("config.toml");
        let (dims_tx, _dims_rx) = tokio::sync::watch::channel((0, 0));
        let (tx, rx) = tokio::sync::mpsc::channel(64);
        let mut stream = ReceiverStream::new(rx);
        let cfg_for_task = config_path.clone();
        let loop_task = tokio::spawn(async move {
            let backend = TestBackend::new(size.0, size.1);
            let mut terminal = ratatui::Terminal::new(backend).expect("terminal");
            multitop::run::event_loop(
                &mut terminal,
                &mut stream,
                dims_tx,
                servers,
                cfg_for_task,
                None,
            )
            .await
        });
        (
            Self {
                _dir: dir,
                cfg: config_path,
                loop_task: Some(loop_task),
            },
            tx,
        )
    }

    /// Take the loop task out for awaiting; the rest of the harness is only the
    /// temp dir and the config path, which live to see the record written.
    const fn finish(&mut self) -> tokio::task::JoinHandle<multitop::run::LoopOutcome> {
        self.loop_task.take().expect("task only taken once")
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.loop_task.abort();
    }
}

impl Drop for PacedHarness {
    fn drop(&mut self) {
        if let Some(task) = self.loop_task.take() {
            task.abort();
        }
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

// Split into a directory target because the single file had reached 619 lines
// and was EXEMPT from the 500-line cap via `.gatesrc`'s GOH_LINE_EXCLUDE while
// also being absent from `tools/loc_baseline.txt` -- so neither the cap nor the
// ratchet applied to it and it could grow without limit. The harness above is
// what all four modules share; the split is by what each one drives.
mod liveness;
mod panel_gen;
mod resize;
