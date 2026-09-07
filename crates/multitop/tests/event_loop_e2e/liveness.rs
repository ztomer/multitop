//! The loop must keep answering keys: after an upgrade ends, while the
//! message channel is flooded, and while a credential lookup blocks. Each of
//! these once read to the user as a dead UI.

use super::*;

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

    let outcome = tokio::time::timeout(Duration::from_secs(15), h.finish())
        .await
        .expect("the loop did not resolve within 15s after `q` -- it is wedged")
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

    let outcome = tokio::time::timeout(Duration::from_secs(15), h.finish())
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

        let outcome = tokio::time::timeout(Duration::from_secs(15), h.finish())
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
