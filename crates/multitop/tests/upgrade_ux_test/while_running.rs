use super::*;

#[tokio::test]
async fn can_return_to_the_pane_while_an_upgrade_is_running() {
    let _keychain = isolate_keychain_async().await;
    let mut h = Harness::new(vec![server("web-01", Some("apt upgrade"))]);
    start_upgrade(&mut h);

    h.press('s');
    assert_eq!(h.app.panels[0].mode, Mode::Monitor);
    assert!(
        h.app.upgrades_in_flight(),
        "leaving the view must not cancel the run"
    );

    h.press('u');
    assert_eq!(
        h.app.panels[0].mode,
        Mode::Upgrade,
        "u must return to the pane while the upgrade is still running"
    );
}

#[tokio::test]
async fn output_produced_while_away_is_shown_on_return() {
    let _keychain = isolate_keychain_async().await;
    let mut h = Harness::new(vec![server("web-01", Some("apt upgrade"))]);
    start_upgrade(&mut h);
    let g = upgrade_gen(&h, 0);

    h.app.apply(Msg::AuxLine {
        panel: 0,
        gen: g,
        line: "while-watching".into(),
    });
    h.press('s');
    h.app.apply(Msg::AuxLine {
        panel: 0,
        gen: g,
        line: "while-away".into(),
    });

    // Upgrade output must not leak into the stats view.
    assert!(
        !h.pane_text(0).contains("while-away"),
        "stats view must not collect upgrade output: {}",
        h.pane_text(0)
    );

    h.press('u');
    let text = h.pane_text(0);
    assert!(text.contains("while-watching"), "{text}");
    assert!(text.contains("while-away"), "{text}");
}

#[tokio::test]
async fn output_keeps_streaming_after_returning_to_the_pane() {
    let _keychain = isolate_keychain_async().await;
    let mut h = Harness::new(vec![server("web-01", Some("apt upgrade"))]);
    start_upgrade(&mut h);
    let g = upgrade_gen(&h, 0);

    h.press('s');
    h.press('u');
    h.app.apply(Msg::AuxLine {
        panel: 0,
        gen: g,
        line: "after-return".into(),
    });

    assert!(
        h.pane_text(0).contains("after-return"),
        "the pane must keep updating after switching back: {}",
        h.pane_text(0)
    );
}

/// The reported symptom: the host that finished while the user was on the
/// stats view showed no completion marker when they came back.
#[tokio::test]
async fn completion_marker_survives_being_away_when_it_arrives() {
    let _keychain = isolate_keychain_async().await;
    let mut h = Harness::new(vec![server("web-01", Some("apt upgrade"))]);
    start_upgrade(&mut h);
    let g = upgrade_gen(&h, 0);

    h.press('s');
    h.app.apply(Msg::AuxDone {
        panel: 0,
        gen: g,
        note: Some("-done".into()),
        success: true,
    });

    assert!(!h.app.upgrades_in_flight(), "the run completed while away");
    assert!(
        !h.pane_text(0).contains("-done"),
        "the marker must not be dumped into the stats view"
    );

    h.press('u');
    let text = h.pane_text(0);
    assert!(
        text.contains("-done"),
        "completion marker must be there on return: {text}"
    );
    assert!(text.contains("ok"), "and the status must read ok: {text}");
}

#[tokio::test]
async fn returning_after_completion_shows_the_finished_state() {
    let _keychain = isolate_keychain_async().await;
    let mut h = Harness::new(vec![server("web-01", Some("apt upgrade"))]);
    start_upgrade(&mut h);
    let g = upgrade_gen(&h, 0);
    h.app.apply(Msg::AuxDone {
        panel: 0,
        gen: g,
        note: Some("-done".into()),
        success: true,
    });

    let text = h.pane_text(0);
    assert!(
        !text.contains("do not quit"),
        "a finished run must stop saying it is running: {text}"
    );
}

// ---------------------------------------------------------------------------
// 6. Reported from a live four-host run: panels showed nothing but
//    "sudo ready", a failing command was blamed on the network, and the status
//    block vanished the moment output arrived.
// ---------------------------------------------------------------------------

/// `Msg::Status` used to assign `view = vec![text]`, throwing away the status
/// header and every line of output collected so far.
#[tokio::test]
async fn a_status_note_does_not_wipe_the_upgrade_pane() {
    let _keychain = isolate_keychain_async().await;
    let mut h = Harness::new(vec![server("web-01", Some("apt upgrade"))]);
    start_upgrade(&mut h);
    let g = upgrade_gen(&h, 0);

    h.app.apply(Msg::AuxLine {
        panel: 0,
        gen: g,
        line: "Reading package lists...".into(),
    });
    h.app.apply(Msg::Status {
        panel: 0,
        gen: g,
        text: "sudo ready - already authorized".into(),
    });

    let text = h.pane_text(0);
    assert!(
        text.contains("apt upgrade"),
        "the status header must survive a status note: {text}"
    );
    assert!(
        text.contains("Reading package lists"),
        "output collected so far must survive a status note: {text}"
    );
    assert!(
        text.contains("sudo ready"),
        "and the note itself belongs in the log: {text}"
    );
}

/// The note must also survive being away, like any other upgrade output.
#[tokio::test]
async fn a_status_note_arriving_while_away_is_kept() {
    let _keychain = isolate_keychain_async().await;
    let mut h = Harness::new(vec![server("web-01", Some("apt upgrade"))]);
    start_upgrade(&mut h);
    let g = upgrade_gen(&h, 0);

    h.app.apply(Msg::Status {
        panel: 0,
        gen: g,
        text: "sudo ready - already authorized".into(),
    });
    h.press('s');
    assert!(
        !h.pane_text(0).contains("sudo ready"),
        "must not be dumped into the stats view"
    );
    h.press('u');
    assert!(h.pane_text(0).contains("sudo ready"), "{}", h.pane_text(0));
}

/// A non-zero exit is a failed command, not a lost connection. Reporting it as
/// "disconnected" pointed at the network for a host the stats view was happily
/// streaming from at that moment.
#[tokio::test]
async fn a_failing_command_is_not_reported_as_a_disconnect() {
    let _keychain = isolate_keychain_async().await;
    let mut h = Harness::new(vec![server("web-01", Some("./update_sys.sh"))]);
    start_upgrade(&mut h);
    let g = upgrade_gen(&h, 0);

    h.app.apply(Msg::AuxDone {
        panel: 0,
        gen: g,
        note: Some("\u{26A0} upgrade command exited 2 - host reachable, command failed".into()),
        success: false,
    });

    let text = h.pane_text(0);
    assert!(text.contains("exited 2"), "must give the exit code: {text}");
    assert!(
        text.contains("host reachable"),
        "must not blame the connection: {text}"
    );
    assert!(
        text.contains("last run failed"),
        "and the badge must say failed: {text}"
    );
}

/// The status block is the point of the pane, so it must not scroll away the
/// moment output starts arriving.
#[tokio::test]
async fn the_status_block_stays_pinned_under_heavy_output() {
    let _keychain = isolate_keychain_async().await;
    let mut h = Harness::new(vec![server("web-01", Some("apt upgrade"))]);
    start_upgrade(&mut h);
    let g = upgrade_gen(&h, 0);
    for i in 0..200 {
        h.app.apply(Msg::AuxLine {
            panel: 0,
            gen: g,
            line: format!("line {i}"),
        });
    }

    // What the renderer would actually show in a 20-row panel: the status
    // header pinned over the ring's tail.
    let header = h.app.upgrade_pane_header(0);
    let (shown, _) = multitop::ui::visible_upgrade(
        &header,
        header.len(),
        &h.app.panels[0].last_upgrade,
        &[],
        20,
        0,
        0,
    );
    let text = strip_ansi(&shown.join("\n"));
    assert!(
        text.contains("Command"),
        "the command must still be visible under 200 lines of output: {text}"
    );
    assert!(
        text.contains("line 199"),
        "and the newest output must still be the tail: {text}"
    );
}

/// A panel whose upgrade never reports back stays "running" for the rest of the
/// session and blocks every later upgrade, because `upgrades_in_flight()` never
/// clears. That is what a failed SSH spawn used to do: it sent a status line and
/// returned, with no `AuxDone`.
#[tokio::test]
async fn a_panel_that_cannot_start_still_reaches_a_terminal_state() {
    let _keychain = isolate_keychain_async().await;
    let mut h = Harness::new(vec![server("web-01", Some("apt upgrade"))]);
    start_upgrade(&mut h);
    let g = upgrade_gen(&h, 0);

    // What the task now emits when ssh::spawn_command fails.
    h.app.apply(Msg::AuxLine {
        panel: 0,
        gen: g,
        line: "ssh: could not resolve hostname".into(),
    });
    h.app.apply(Msg::AuxDone {
        panel: 0,
        gen: g,
        note: Some("\u{26A0} could not start the upgrade over SSH".into()),
        success: false,
    });

    assert!(
        !h.app.upgrades_in_flight(),
        "a panel that could not start must not block every later upgrade"
    );
    let text = h.pane_text(0);
    assert!(
        !text.contains("do not quit"),
        "and it must stop claiming to be running: {text}"
    );
    assert!(text.contains("could not start"), "{text}");

    // The user can immediately try again.
    h.press('u');
    assert!(
        h.app.show_upgrade_modal(),
        "u must be able to arm another attempt"
    );
}

/// `AuxBegin` arrives immediately after every upgrade starts. It used to
/// replace the whole view, so the status header was destroyed on every single
/// run before a byte of output appeared -- leaving a bare "Upgrade on <host>"
/// line that the panel banner then overwrote, which is why panels showed
/// output with no header at all.
#[tokio::test]
async fn the_pane_survives_the_aux_begin_that_every_run_sends() {
    let _keychain = isolate_keychain_async().await;
    let mut h = Harness::new(vec![server("web-01", Some("apt upgrade"))]);
    start_upgrade(&mut h);
    let g = upgrade_gen(&h, 0);

    h.app.apply(Msg::AuxBegin {
        panel: 0,
        gen: g,
        header: Some("Upgrade on web-01".into()),
    });

    let text = h.pane_text(0);
    assert!(
        text.contains("apt upgrade"),
        "the status header must survive AuxBegin: {text}"
    );
    assert!(
        text.contains("running"),
        "and still show the running state: {text}"
    );
}

/// Reported from live use: the pane said "will prompt" for a host whose
/// password had been saved. Passwords load lazily, so a panel that has not run
/// an upgrade yet holds nothing in memory, and the pane read that emptiness as
/// "no password" instead of asking the store.
#[tokio::test]
async fn the_pane_reports_a_saved_password_rather_than_promising_a_prompt() {
    let _guard = multitop::password_store::lock_for_test_async().await;
    multitop::password_store::enable_mock_store();
    multitop::password_store::clear_mock_store();

    let s = server("web-01", Some("apt upgrade"));
    multitop::password_store::save(&s, "stored-secret").unwrap();

    let mut h = Harness::new(vec![s]);
    // Nothing loaded yet: exactly the state a fresh session is in.
    assert!(h.app.panels[0].sudo_password.is_none());

    h.press('u');

    let text = h.pane_text(0);
    assert!(
        text.contains("password stored"),
        "a saved password must be reported as stored: {text}"
    );
    assert!(
        !text.contains("will prompt"),
        "and must not threaten a prompt that will not happen: {text}"
    );
}
