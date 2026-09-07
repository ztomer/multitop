//! Monitor packets must carry the panel generation current AFTER a view
//! switch, or `apply.rs` rejects them.

use super::*;

/// Regression for the `gen` tracking bug: after a view switch (upgrade/docker/fetch),
/// `panel.gen` increments. Monitor packets must carry the new gen, or `apply.rs`
/// rejects them. The bug was `spawn_monitor` hardcoding `gen: 0`.
#[tokio::test]
async fn monitor_packets_use_the_panel_gen_after_a_view_switch() {
    let _keychain = isolate_keychain().await;
    let servers = vec![local_server(42025, "echo hi")];
    let (mut h, tx) = PacedHarness::start(servers, (80, 24));

    // Enter the upgrade view -- this increments panel.gen -- then confirm.
    tx.send(Ok(key(KeyCode::Char('u')))).await.expect("key");
    tokio::time::sleep(Duration::from_millis(300)).await;
    for _ in 0..2 {
        tx.send(Ok(key(KeyCode::Char('u')))).await.expect("key");
    }

    // Wait for the run on the durable record, the way its sibling above does,
    // rather than on a fixed sleep.
    //
    // This test used to sleep 500 ms and then send a single `q`, and it failed
    // roughly one run in three under a loaded workspace. Two things made that
    // fragile, and only one of them is about load. Quitting while an upgrade is
    // *in flight* deliberately takes two presses -- `request_quit` arms and the
    // confirm row wants a second `q` -- so the test was only ever passing
    // because the run happened to be over by the time it pressed. Whether it
    // was is a race between a sleep and a subprocess.
    //
    // It also got slower for a reason worth writing down: the upgrade now runs
    // through the agent under an interactive login shell, the same one the
    // remote path always used, so a local run pays the rc-file startup a remote
    // one always did. That is the price of local and remote behaving alike, and
    // it is milliseconds against an upgrade that takes minutes -- but it moved
    // this test across the line it was already sitting on.
    let account = "admin@127.0.0.1:42025";
    let recorded = tokio::time::timeout(Duration::from_secs(20), async {
        while !state_says_finished(&h.cfg, account) {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    assert!(
        recorded.is_ok(),
        "the upgrade never recorded a finished_at on disk -- it did not run, or \
         the loop is not applying AuxDone"
    );

    // Switch back to the monitor view: this increments gen again. If the
    // monitor task were respawned with a stale gen its packets would be
    // rejected and the panel would show nothing -- and the loop resolving
    // cleanly below is the signal that it is still processing.
    tx.send(Ok(key(KeyCode::Char('s')))).await.expect("key");
    tx.send(Ok(key(KeyCode::Char('q')))).await.expect("key");

    let outcome = tokio::time::timeout(Duration::from_secs(15), h.finish())
        .await
        .expect("loop wedged after view switch -- monitor gen likely stale")
        .expect("loop task panicked");
    assert!(
        outcome.error.is_none(),
        "loop ended with error: {:?}",
        outcome.error
    );
}
