//! A monitor task must not paint a panel that now belongs to a different host.
//!
//! `spawn_monitor` captures its panel index once and loops forever, reconnecting
//! on failure. Editing the server list replaces `app.panels` but does not stop
//! those tasks, so the task started for the old index 0 keeps sending frames for
//! index 0 -- which, after a deletion, is a different machine. Every other
//! message type is gated on `accepts(panel, gen)`; `Msg::Frame` was gated on the
//! index alone, so the stale frames landed and each remaining panel showed
//! another host's statistics under its own name.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use multitop::app::{App, Msg};
use multitop::config::Server;

fn server(host: &str) -> Server {
    Server {
        host: host.to_string(),
        port: 22,
        user: "ztomer".to_string(),
        upgrade_cmd: None,
    }
}

#[test]
fn a_frame_from_a_replaced_panel_is_rejected() {
    let mut app = App::new(vec![server("host-a"), server("host-b"), server("host-c")]);

    // A frame from the task that owns panel 0 right now is applied.
    let epoch0 = app.panels_epoch;
    app.apply(Msg::Frame {
        panel: 0,
        epoch: epoch0,
        lines: vec!["A-STATS".to_string()],
    });
    assert_eq!(
        app.panels[0].last_frame.as_deref(),
        Some(["A-STATS".to_string()].as_slice()),
        "a live task's frame must still be applied"
    );

    // The user deletes host-a. This is what ApplyServers does: build fresh
    // panels for the new list. host-b now occupies index 0.
    app.replace_panels(vec![server("host-b"), server("host-c")]);
    assert_eq!(app.panels[0].server.host, "host-b");

    // host-a's monitor task has not been stopped and keeps sending for index 0.
    app.apply(Msg::Frame {
        panel: 0,
        epoch: epoch0,
        lines: vec!["A-STATS-LATER".to_string()],
    });

    assert_ne!(
        app.panels[0].last_frame.as_deref(),
        Some(["A-STATS-LATER".to_string()].as_slice()),
        "host-a's frame was painted onto host-b's panel"
    );
}

#[test]
fn a_frame_from_the_current_task_is_still_applied_after_a_change() {
    let mut app = App::new(vec![server("host-a"), server("host-b")]);
    app.replace_panels(vec![server("host-b")]);

    let epoch_now = app.panels_epoch;
    app.apply(Msg::Frame {
        panel: 0,
        epoch: epoch_now,
        lines: vec!["B-STATS".to_string()],
    });

    assert_eq!(
        app.panels[0].last_frame.as_deref(),
        Some(["B-STATS".to_string()].as_slice()),
        "the panel must still accept frames from a task spawned for the new list"
    );
}

#[test]
fn replacing_panels_advances_the_epoch() {
    let mut app = App::new(vec![server("host-a"), server("host-b")]);
    let before = app.panels_epoch;

    app.replace_panels(vec![server("host-a"), server("host-b")]);

    assert_ne!(
        app.panels_epoch, before,
        "the epoch did not move, so a task holding the old one still matches"
    );
}

/// The epoch must survive a mode switch, or the guard would freeze the panel.
///
/// `gen` is bumped by every mode change, so gating frames on it rejected every
/// monitor frame after the user first pressed `d` -- the stats view would stop
/// updating for the rest of the session. Two of the existing app tests caught
/// exactly that.
#[test]
fn switching_modes_does_not_retire_the_running_monitor() {
    let mut app = App::new(vec![server("host-a")]);
    let epoch = app.panels_epoch;

    app.toggle_docker();
    app.switch_stats();

    app.apply(Msg::Frame {
        panel: 0,
        epoch,
        lines: vec!["STILL-LIVE".to_string()],
    });
    assert_eq!(
        app.panels[0].last_frame.as_deref(),
        Some(["STILL-LIVE".to_string()].as_slice()),
        "a mode switch must not stop the monitor task's frames being accepted"
    );
}
