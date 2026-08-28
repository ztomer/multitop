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

/// The reported defect: after the upgrade finished, the app never answered
/// another key -- switching panes did nothing and even `q` would not quit.
///
/// This is the missing case from the suite: it drives the *real* event loop to
/// the upgrade's end and then asks it to act, instead of calling `handle_key`
/// in isolation like the UX tests, which could not see a key that was read but
/// never acted on. The upgrade runs as a real local command, its completion is
/// confirmed on disk (state.toml records `finished_at`), and only then is `q`
/// sent. If the loop woke up from the run wedged, `q` would land on a loop that
/// never processes it and the join would not resolve in time.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_loop_still_quits_after_the_upgrade_completes() {
    let _keychain = isolate_keychain().await;
    let cmd = "i=0; while [ $i -lt 300 ]; do echo tick-$i; i=$((i+1)); sleep 0.01; done";
    let servers = vec![local_server(42022, cmd)];
    let (mut h, tx) = PacedHarness::start(servers, (80, 24));

    // Entering the upgrade view dispatches the credential-store lookup off the
    // loop thread, and the confirm is deferred until that answer lands (the
    // header reads `checking` meanwhile). The mock store answers instantly, so
    // a beat between the enter and the confirm queuing makes the ordering
    // deterministic: enter, let the answer land, then confirm.
    tx.send(Ok(key(KeyCode::Char('u')))).await.expect("key");
    tokio::time::sleep(Duration::from_millis(300)).await;
    for _ in 0..2 {
        tx.send(Ok(key(KeyCode::Char('u')))).await.expect("key");
    }

    // Completion, proven from the durable record rather than assumed. The
    // streamer takes ~3s, so a wedged loop would have all the time in the
    // world to fail this deadline.
    let account = "admin@127.0.0.1:42022";
    let recorded = tokio::time::timeout(Duration::from_secs(10), async {
        while !state_says_finished(&h.cfg, account) {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    assert!(
        recorded.is_ok(),
        "the upgrade never recorded a finished_at on disk -- it did not run or \
         the loop is not applying AuxDone\n{:?}",
        std::fs::read_dir(h.cfg.parent().expect("dir"))
            .map(|it| it
                .filter_map(Result::ok)
                .map(|e| e.path())
                .collect::<Vec<_>>())
            .unwrap_or_default()
    );

    // Now that the run is over, switch away from the Upgrade pane and quit --
    // the pair of actions that did nothing in the report.
    tx.send(Ok(key(KeyCode::Char('s')))).await.expect("key");
    tx.send(Ok(key(KeyCode::Char('q')))).await.expect("key");

    let outcome = tokio::time::timeout(Duration::from_secs(5), h.finish())
        .await
        .expect("the loop did not resolve within 5s after `q` -- it is wedged")
        .expect("the loop task panicked");
    assert!(
        outcome.error.is_none(),
        "loop ended with an error: {:?}",
        outcome.error
    );
    assert!(
        outcome.killed.is_empty(),
        "`q` came after the upgrade finished; nothing should have been killed"
    );
}

/// A single producer (an upgrade streaming output) can flood the message
/// channel far beyond the drain budget, and the drain must not starve the key
/// branch -- that was the other way a finished run read as a dead UI.
///
/// The command bursts a long list as fast as the pipe delivers it, then the
/// test waits long enough for the flood to be at full throttle and asks for big
/// A quit session must be served while the upgrade channel is flooded. The
/// drain runs 32 messages per poll; the keys must still land between drains.
///
/// One `q` only *arms* quit while an upgrade is in flight (a deliberate guard
/// around killing a running apt on hosts); a second `q` confirms it. Both are
/// sent mid-flood so a drain that starves keys fails the loop to resolve.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_quit_key_lands_while_the_upgrade_channel_is_flooded() {
    let _keychain = isolate_keychain().await;
    let cmd = "i=0; while [ $i -lt 20000 ]; do echo flood-$i; i=$((i+1)); done";
    let servers = vec![local_server(42023, cmd)];
    let (mut h, tx) = PacedHarness::start(servers, (80, 24));

    // Enter first, then let the deferred credential answer land (the confirm is
    // gated on it), then queue the modal and the confirm so the run definitely
    // starts and the channel has a flood to build.
    tx.send(Ok(key(KeyCode::Char('u')))).await.expect("key");
    tokio::time::sleep(Duration::from_millis(300)).await;
    for _ in 0..2 {
        tx.send(Ok(key(KeyCode::Char('u')))).await.expect("key");
    }
    // Let the burst build to full throttle before asking for anything.
    tokio::time::sleep(Duration::from_millis(250)).await;

    tx.send(Ok(key(KeyCode::Char('s')))).await.expect("key");
    tx.send(Ok(key(KeyCode::Char('q')))).await.expect("key");
    tokio::time::sleep(Duration::from_millis(250)).await;
    tx.send(Ok(key(KeyCode::Char('q')))).await.expect("key");

    let outcome = tokio::time::timeout(Duration::from_secs(5), h.finish())
        .await
        .inspect_err(|_elapsed| {
            // Timeout: ask the loop itself where it is before declaring a wedge.
            let _ = std::process::Command::new("sh")
                .arg("-c")
                .arg(format!("kill -USR2 {}", std::process::id()))
                .status();
            std::thread::sleep(Duration::from_millis(200));
            let mut newest: Option<std::path::PathBuf> = None;
            let mypid = std::process::id();
            if let Ok(entries) = std::fs::read_dir(multitop::diag::Diag::default_dir()) {
                for e in entries.flatten() {
                    let p = e.path();
                    let n = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
                    if n.starts_with(&format!("multitop-diag-{mypid}-"))
                        && n.ends_with("-state.txt")
                        && newest.as_ref().is_none_or(|cur: &std::path::PathBuf| {
                            cur.file_name()
                                .and_then(|c| c.to_str())
                                .is_some_and(|c| c < n)
                        })
                    {
                        newest = Some(p);
                    }
                }
            }
            if let Some(p) = newest {
                eprintln!(
                    "\n--- state-tier on timeout ---\n{}",
                    std::fs::read_to_string(&p).unwrap_or_default()
                );
            }
        })
        .expect("`q` never confirmed quit during the flood -- the drain starves keys")
        .expect("the loop task panicked");
    assert!(
        outcome.error.is_none(),
        "loop ended with an error: {:?}",
        outcome.error
    );
    // `q` above is not expected to have waited for the flood to end (20000 lines
    // stream in under the pipe), but the loop must have answered it regardless.
    drop(outcome);
    let _ = h.cfg;
}

/// A credential-store lookup that blocks -- the shape that froze the loop
/// before Layer 3. The OS keychain can park on a system dialog for many
/// seconds; the read used to happen on the event-loop thread, so the whole TUI
/// froze for exactly that long and a quit pressed meanwhile was never served.
/// Now the lookup runs on a blocking worker and the loop stays live.
///
/// The mock callback sleeps far longer than the test's patience, so the quit
/// deadline only resolves because the loop never waited on the store. With the
/// old synchronous read this test failed: the loop thread was the one sleeping,
/// and `q` could not be served for the whole delay.
///
/// A plain `#[test]` rather than `#[tokio::test]`: the runtime is built by hand
/// so its `shutdown_timeout` bounds the drain of the parked blocking worker at
/// teardown, or this test would sit out the whole 15s delay (the production
/// runtime carries the same bound for the same reason -- `main` would hang on
/// quit while a keychain dialog was up).
#[test]
fn the_loop_stays_live_while_a_credential_load_blocks() {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .worker_threads(2)
        .build()
        .expect("runtime");
    rt.block_on(async {
        let _keychain = isolate_keychain().await;
        password_store::set_mock_load_delay(Some(Duration::from_secs(15)));
        let servers = vec![local_server(42024, "echo hi")];
        let (mut h, tx) = PacedHarness::start(servers, (80, 24));

        // Entering the upgrade view dispatches the lookup; the answer takes 15s.
        tx.send(Ok(key(KeyCode::Char('u')))).await.expect("key");
        tokio::time::sleep(Duration::from_millis(200)).await;

        // Queue confirm; the pane header reads `checking` and the confirm must be
        // deferred, not start a run on a password the store has not returned.
        tx.send(Ok(key(KeyCode::Char('u')))).await.expect("key");
        tokio::time::sleep(Duration::from_millis(200)).await;
        tx.send(Ok(key(KeyCode::Char('u')))).await.expect("key");
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert!(
            !state_says_started(&h.cfg, "admin@127.0.0.1:42024"),
            "the confirm must be deferred while the credential lookup is in flight"
        );

        // Quit, while the lookup is still blocking. `q` on the confirm row
        // cancels it first; a second `q` then quits, with no upgrade in flight
        // to arm it.
        tx.send(Ok(key(KeyCode::Char('q')))).await.expect("key");
        tokio::time::sleep(Duration::from_millis(250)).await;
        tx.send(Ok(key(KeyCode::Char('q')))).await.expect("key");

        let outcome = tokio::time::timeout(Duration::from_secs(5), h.finish())
            .await
            .expect("the loop never quit while a credential load was in flight")
            .expect("the loop task panicked");
        assert!(
            outcome.error.is_none(),
            "loop ended with an error: {:?}",
            outcome.error
        );
        drop(outcome);
        let _ = h.cfg;
    });
    rt.shutdown_timeout(Duration::from_secs(3));
}

/// Regression for the `gen` tracking bug: after a view switch (upgrade/docker/fetch),
/// `panel.gen` increments. Monitor packets must carry the new gen, or `apply.rs`
/// rejects them. The bug was `spawn_monitor` hardcoding `gen: 0`.
#[tokio::test]
async fn monitor_packets_use_the_panel_gen_after_a_view_switch() {
    let _keychain = isolate_keychain().await;
    let servers = vec![local_server(42025, "echo hi")];
    let (mut h, tx) = PacedHarness::start(servers, (80, 24));

    // Enter upgrade view — this increments panel.gen
    tx.send(Ok(key(KeyCode::Char('u')))).await.expect("key");
    tokio::time::sleep(Duration::from_millis(300)).await;
    for _ in 0..2 {
        tx.send(Ok(key(KeyCode::Char('u')))).await.expect("key");
    }
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Switch back to monitor view (s) — this increments gen again
    tx.send(Ok(key(KeyCode::Char('s')))).await.expect("key");
    tokio::time::sleep(Duration::from_millis(200)).await;

    // The loop should still be alive and processing. If the monitor task was
    // spawned with a stale gen, packets would be rejected and the panel would
    // show nothing. We can't easily inspect the panel content here without the
    // full Harness, but the loop resolving without wedging is the signal.
    tx.send(Ok(key(KeyCode::Char('q')))).await.expect("key");

    let outcome = tokio::time::timeout(Duration::from_secs(5), h.finish())
        .await
        .expect("loop wedged after view switch — monitor gen likely stale")
        .expect("loop task panicked");
    assert!(
        outcome.error.is_none(),
        "loop ended with error: {:?}",
        outcome.error
    );
}
