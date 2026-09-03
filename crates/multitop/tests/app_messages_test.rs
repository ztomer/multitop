//! Messages the app has to act on, and the ones it has to ignore.
//!
//! Every arm here is "a packet arrived — does it change what is on screen?".
//! Getting that wrong is either a flickering idle TUI (redrawing for nothing)
//! or a stale one (a frame that never lands), and neither shows up as a crash.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use multitop::app::{App, Mode, Msg, VaultState};
use multitop::config::Server;
use multitop::panel::UpgradeState;
use multitop::password_store;
use multitop_agent::fetch::FetchSnapshot;
use multitop_agent::proc::{Proc, Usage};
use multitop_agent::proto::Payload;
use multitop_agent::render::Snapshot;

const DIMS: (u16, u16) = (80, 24);

fn test_server(host: &str) -> Server {
    Server {
        host: host.to_string(),
        port: 0,
        user: "admin".to_string(),
        upgrade_cmd: Some("true".to_string()),
        custom_command: None,
    }
}

async fn isolate() -> tokio::sync::MutexGuard<'static, ()> {
    let guard = password_store::lock_for_test_async().await;
    password_store::enable_mock_store();
    password_store::clear_mock_store();
    guard
}

fn test_app(hosts: &[&str]) -> App {
    App::new(hosts.iter().map(|h| test_server(h)).collect())
}

fn monitor_payload(host: &str) -> Payload {
    Payload::Monitor(Snapshot {
        host: host.into(),
        agent_version: "9.9.9".into(),
        cpu_pct: 30.0,
        cpu_mhz: Some(3600.0),
        proc_names: Vec::new(),
        cores: vec![(0, 10.0, None)],
        mem: Usage::new(8 << 30, 2 << 30),
        disk: Usage::new(256 << 30, 64 << 30),
        rx_rate: 1.0,
        tx_rate: 1.0,
        procs: vec![Proc {
            pid: 1,
            name: "init".into(),
            cpu: 1.0,
            mem: 1024,
        }],
        ..Default::default()
    })
}

fn docker_payload() -> Payload {
    Payload::Docker {
        host: "web-01".into(),
        rows: vec![multitop_agent::docker::Row {
            name: "web".into(),
            status: "Up".into(),
            image: "nginx:latest".into(),
            cpu: "1.0%".into(),
            cpu_pct: 1.0,
            mem: "-".into(),
            mem_bytes: 0,
        }],
    }
}

fn fetch_snapshot() -> FetchSnapshot {
    FetchSnapshot {
        user_host: "root@web-01".into(),
        agent_version: "9.9.9".into(),
        os: "Debian".into(),
        kernel: "6.1".into(),
        uptime: "1d 0h 0m".into(),
        host_model: "QEMU".into(),
        cpu_model: "AMD (8)".into(),
        memory_str: "2.0G/8.0G".into(),
        disk_str: "64G/256G".into(),
    }
}

const fn packet(panel: usize, gen: u64, payload: Payload) -> Msg {
    Msg::Packet {
        panel,
        gen,
        epoch: 0,
        payload,
        dims: DIMS,
    }
}

// ------------------------------------------------------------------- packets

#[tokio::test]
async fn a_monitor_packet_always_changes_the_screen() {
    // The banner host name is drawn on every panel whatever view it is in, so
    // a monitor packet is never a no-op.
    let _g = isolate().await;
    let mut app = test_app(&["alpha"]);
    let gen = app.panels[0].gen;

    for mode in [Mode::Monitor, Mode::Docker, Mode::Fetch, Mode::Upgrade] {
        app.panels[0].mode = mode;
        assert!(
            app.apply(packet(0, gen, monitor_payload("alpha"))),
            "a monitor packet was discarded in {mode:?}"
        );
    }
    assert!(app.panels[0].last_monitor.is_some());
    assert!(
        app.panels[0].last_frame.is_some(),
        "no frame was cached to re-render from"
    );
}

#[tokio::test]
async fn a_docker_packet_is_cached_always_and_drawn_only_in_the_docker_view() {
    let _g = isolate().await;
    let mut app = test_app(&["alpha"]);
    let gen = app.panels[0].gen;

    app.panels[0].mode = Mode::Monitor;
    assert!(
        !app.apply(packet(0, gen, docker_payload())),
        "a docker packet redrew a monitor panel"
    );
    assert!(
        app.panels[0].last_docker.is_some(),
        "the payload was not cached"
    );

    app.panels[0].mode = Mode::Docker;
    assert!(app.apply(packet(0, gen, docker_payload())));
}

#[tokio::test]
async fn a_fetch_packet_is_cached_always_and_drawn_only_in_the_fetch_view() {
    let _g = isolate().await;
    let mut app = test_app(&["alpha"]);
    let gen = app.panels[0].gen;

    app.panels[0].mode = Mode::Monitor;
    assert!(!app.apply(packet(0, gen, Payload::Fetch(fetch_snapshot()))));
    assert!(app.panels[0].last_fetch.is_some());

    app.panels[0].mode = Mode::Fetch;
    assert!(app.apply(packet(0, gen, Payload::Fetch(fetch_snapshot()))));
}

#[tokio::test]
async fn a_packet_for_a_panel_that_is_not_there_is_dropped() {
    let _g = isolate().await;
    let mut app = test_app(&["alpha"]);
    assert!(!app.apply(packet(9, 0, monitor_payload("ghost"))));
}

#[tokio::test]
async fn a_packet_from_a_retired_generation_is_dropped() {
    // A view switch bumps the generation; frames from the old one are answers
    // to a question nobody is asking any more.
    let _g = isolate().await;
    let mut app = test_app(&["alpha"]);
    app.panels[0].mode = Mode::Docker;
    let stale = app.panels[0].gen;
    app.bump(0);

    assert!(!app.apply(packet(0, stale, docker_payload())));
}

#[tokio::test]
async fn a_prerendered_fetch_frame_lands_when_it_is_still_wanted() {
    let _g = isolate().await;
    let mut app = test_app(&["alpha"]);
    let gen = app.panels[0].gen;

    assert!(app.apply(Msg::FetchData {
        panel: 0,
        gen,
        snap: fetch_snapshot(),
        lines: vec!["rendered".to_string()],
    }));
    assert!(app.panels[0].last_fetch.is_some());

    // And is dropped once the generation has moved on.
    app.bump(0);
    assert!(!app.apply(Msg::FetchData {
        panel: 0,
        gen,
        snap: fetch_snapshot(),
        lines: vec!["stale".to_string()],
    }));
}

// ---------------------------------------------------------------- vault msgs

#[tokio::test]
async fn a_rotation_result_is_reported_to_the_user_either_way() {
    let _g = isolate().await;
    let mut app = test_app(&["alpha"]);
    let epoch = app.vault_epoch;

    assert!(app.apply(Msg::VaultPasswordRotated { epoch }));
    let after_success = app.panels[0].notes.join("\n");
    assert!(after_success.contains("changed"), "{after_success}");

    let mut app2 = test_app(&["alpha"]);
    let epoch = app2.vault_epoch;
    assert!(app2.apply(Msg::VaultPasswordRotationFailed {
        epoch,
        error: "wrong current password".into(),
    }));
    let after_failure = app2.panels[0].notes.join("\n");
    // The useful fact is that nothing changed.
    assert!(after_failure.contains("NOT changed"), "{after_failure}");
    assert!(
        after_failure.contains("wrong current password"),
        "{after_failure}"
    );
}

#[tokio::test]
async fn a_rotation_result_from_a_retired_epoch_is_ignored() {
    let _g = isolate().await;
    let mut app = test_app(&["alpha"]);
    let stale = app.vault_epoch;
    app.bump_vault_epoch();

    assert!(!app.apply(Msg::VaultPasswordRotated { epoch: stale }));
    assert!(!app.apply(Msg::VaultPasswordRotationFailed {
        epoch: stale,
        error: "late".into(),
    }));
    assert!(
        !app.panels[0].notes.join("\n").contains("Master password"),
        "a stale rotation result reached the panel"
    );
}

#[tokio::test]
async fn a_biometric_failure_falls_through_to_the_password_prompt() {
    let _g = isolate().await;
    let mut app = test_app(&["alpha"]);
    app.vault_state = VaultState::Unlocking {
        awaiting_biometric: true,
    };
    let epoch = app.vault_epoch;

    assert!(app.apply(Msg::VaultBiometricFailed { epoch }));
    assert!(
        app.show_vault_password_prompt(),
        "a failed biometric left the user with nothing to do"
    );
}

// -------------------------------------------------------------- view switches

#[tokio::test]
async fn re_entering_a_view_redraws_from_the_cached_payload() {
    // Showing "loading..." on re-entry with the data already in hand is what
    // made a view switch look like a dropped connection.
    let _g = isolate().await;
    let mut app = test_app(&["alpha"]);
    let gen = app.panels[0].gen;

    app.panels[0].mode = Mode::Docker;
    app.apply(packet(0, gen, docker_payload()));
    app.panels[0].mode = Mode::Monitor;
    app.apply(packet(0, app.panels[0].gen, monitor_payload("alpha")));
    app.panels[0].mode = Mode::Fetch;
    app.apply(packet(
        0,
        app.panels[0].gen,
        Payload::Fetch(fetch_snapshot()),
    ));

    // `rerender_all` walks every panel in whatever view it is in and redraws
    // from what it already has.
    for mode in [Mode::Monitor, Mode::Docker, Mode::Fetch, Mode::Upgrade] {
        app.panels[0].mode = mode;
        app.panels[0].view.clear();
        app.rerender_all((100, 30));
        if mode == Mode::Upgrade {
            continue;
        }
        assert!(
            !app.panels[0].view.is_empty(),
            "re-entering {mode:?} drew nothing despite cached data"
        );
    }
}

#[tokio::test]
async fn asking_for_the_view_you_are_already_in_costs_nothing() {
    let _g = isolate().await;
    let mut app = test_app(&["alpha"]);

    assert!(
        !app.toggle_fetch(DIMS).is_empty(),
        "the first switch must spawn work"
    );
    assert!(app.in_fetch());
    assert!(
        app.toggle_fetch(DIMS).is_empty(),
        "switching to the view already showing re-spawned every agent"
    );

    assert_ne!(app.toggle_docker(DIMS), [] as [multitop::app::Command; 0]);
    assert!(app.in_docker());
    assert_eq!(app.toggle_docker(DIMS), [] as [multitop::app::Command; 0]);
}

// ---------------------------------------------------------------------- quit

#[tokio::test]
async fn quitting_with_an_upgrade_running_asks_first_and_then_goes() {
    let _g = isolate().await;
    let mut app = test_app(&["alpha"]);
    app.panels[0].upgrade_state = UpgradeState::STARTED;

    app.request_quit();
    assert!(
        !app.should_quit(),
        "a running upgrade was killed without asking"
    );
    app.request_quit();
    assert!(app.should_quit(), "the second ask did not go through");
}

#[tokio::test]
async fn quitting_with_nothing_running_goes_straight_out() {
    let _g = isolate().await;
    let mut app = test_app(&["alpha"]);
    app.request_quit();
    assert!(
        app.should_quit(),
        "an idle app asked a question with no stakes"
    );
}

// ------------------------------------------------------------ upgrade tracking

#[tokio::test]
async fn a_host_with_no_upgrade_command_is_not_recorded_as_having_started_one() {
    let _g = isolate().await;
    let mut app = App::new(vec![
        Server {
            host: "alpha".into(),
            port: 0,
            user: "a".into(),
            upgrade_cmd: Some("true".into()),
            custom_command: None,
        },
        Server {
            host: "beta".into(),
            port: 0,
            user: "a".into(),
            upgrade_cmd: None,
            custom_command: None,
        },
    ]);

    app.mark_upgrades_started(&[0, 1]);
    assert_eq!(
        app.host_updates.len(),
        1,
        "a host that cannot upgrade was recorded as upgrading: {:?}",
        app.host_updates.keys().collect::<Vec<_>>()
    );

    // An index past the end is skipped rather than panicking.
    app.mark_upgrades_started(&[99]);
}

#[tokio::test]
async fn only_a_panel_that_was_actually_running_is_marked_interrupted() {
    let _g = isolate().await;
    let mut app = test_app(&["alpha"]);

    // Not started: nothing to interrupt.
    app.mark_upgrade_interrupted(0);
    assert_ne!(app.panels[0].upgrade_state, UpgradeState::STARTED);

    app.panels[0].upgrade_state = UpgradeState::STARTED;
    app.mark_upgrade_interrupted(0);
    assert_ne!(
        app.panels[0].upgrade_state,
        UpgradeState::STARTED,
        "an interrupted upgrade stayed 'running' forever"
    );

    // A panel that is not there is skipped rather than panicking.
    app.mark_upgrade_interrupted(99);
}

// ----------------------------------------------- messages for absent panels

#[tokio::test]
async fn a_message_for_a_panel_that_is_no_longer_there_is_dropped() {
    // A task started for the old panel list can outlive an edit. Indexing with
    // its captured index would paint whichever host moved into that slot.
    let _g = isolate().await;
    let mut app = test_app(&["alpha"]);
    let gen = app.panels[0].gen;

    assert!(!app.apply(Msg::AuxLine {
        panel: 9,
        gen,
        line: "output from a host that is gone".into(),
    }));
    assert!(!app.apply(Msg::AuxBegin {
        panel: 9,
        gen,
        header: Some("header".into()),
    }));
    assert!(!app.apply(Msg::AuxDone {
        panel: 9,
        gen,
        note: Some("done".into()),
        success: true,
    }));
    // Nothing reached the panel that is there.
    assert!(app.panels[0].last_upgrade.is_empty());
}

#[tokio::test]
async fn an_unlock_from_a_retired_attempt_is_ignored() {
    // The user cancelled and started again; the first attempt's answer must
    // not open a vault they have moved on from.
    let _g = isolate().await;
    let mut app = test_app(&["alpha"]);
    let stale = app.vault_epoch;
    app.bump_vault_epoch();

    assert!(!app.apply(Msg::VaultUnlockFailed {
        epoch: stale,
        error: "too late".into(),
    }));
    assert!(!app.apply(Msg::VaultBiometricFailed { epoch: stale }));
    assert!(
        !app.show_vault_password_prompt(),
        "a retired attempt raised a prompt"
    );
}
