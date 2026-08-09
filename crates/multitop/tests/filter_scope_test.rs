//! `/` searches what the panel is showing, and nothing else.
//!
//! The filter used to match two fixed fields -- the configured host and user --
//! so a process name, a container, an image or an OS version could be on screen
//! and unfindable. It now asks each panel, and each panel answers from the view
//! it is currently in: searching a Docker payload while the user is looking at
//! the stats table would give a different answer depending on where they had
//! been, which is worse than not searching it at all.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use multitop::app::{App, Msg};
use multitop::config::Server;
use multitop::panel::{Mode, Panel};
use multitop::password_store;
use multitop::run::{handle_key, Tasks};
use multitop_agent::docker::Row as DockerRow;
use multitop_agent::fetch::FetchSnapshot;
use multitop_agent::proc::{Proc, Usage};
use multitop_agent::proto::Payload;
use multitop_agent::render::Snapshot;
use std::sync::Arc;
use tokio::sync::{mpsc, watch};

fn test_server(host: &str) -> Server {
    Server {
        host: host.to_string(),
        port: 22,
        user: "deploy".to_string(),
        upgrade_cmd: Some("true".to_string()),
    }
}

async fn isolate() -> tokio::sync::MutexGuard<'static, ()> {
    let guard = password_store::lock_for_test_async().await;
    password_store::enable_mock_store();
    password_store::clear_mock_store();
    guard
}

fn monitor(reported_host: &str, procs: &[&str]) -> Payload {
    Payload::Monitor(Snapshot {
        host: reported_host.into(),
        agent_version: "9.9.9".into(),
        cpu_pct: 10.0,
        cpu_mhz: Some(3600.0),
        cores: vec![(0, 10.0, None)],
        mem: Usage::new(100, 40),
        disk: Usage::new(100, 10),
        rx_rate: 0.0,
        tx_rate: 0.0,
        procs: procs
            .iter()
            .enumerate()
            .map(|(i, name)| Proc {
                pid: u32::try_from(i).unwrap() + 1,
                name: (*name).to_string(),
                cpu: 1.0,
                mem: 1024,
            })
            .collect(),
        ..Default::default()
    })
}

fn docker(containers: &[(&str, &str)]) -> Payload {
    Payload::Docker {
        host: "docker-host".into(),
        rows: containers
            .iter()
            .map(|(name, image)| DockerRow {
                name: (*name).to_string(),
                status: "Up 3 hours".to_string(),
                image: (*image).to_string(),
                cpu: "1.0%".to_string(),
                cpu_pct: 1.0,
                mem: "-".to_string(),
                mem_bytes: 0,
            })
            .collect(),
    }
}

fn fetch(os: &str, model: &str) -> FetchSnapshot {
    FetchSnapshot {
        user_host: "deploy@web-01".into(),
        agent_version: "9.9.9".into(),
        os: os.into(),
        kernel: "6.8.0-45-generic".into(),
        uptime: "3d 4h".into(),
        host_model: model.into(),
        cpu_model: "AMD EPYC 7763".into(),
        memory_str: "64G".into(),
        disk_str: "1T".into(),
    }
}

/// One panel, loaded with every payload, sitting in `mode`.
fn loaded_panel(host: &str, mode: Mode) -> Panel {
    let mut p = Panel::new(test_server(host));
    p.mode = mode;
    p.last_monitor = Some(monitor("reported-name", &["postgres", "nginx", "sshd"]));
    p.last_docker = Some(docker(&[
        ("billing-api", "registry.example.com/team/billing:2.1"),
        ("redis", "redis:7-alpine"),
    ]));
    p.last_fetch = Some(fetch("Ubuntu 24.04.1 LTS", "PowerEdge R650"));
    p.last_upgrade.push("Setting up libssl3:amd64".to_string());
    p
}

// ---------------------------------------------------------- always searchable

#[test]
fn the_host_and_user_match_in_every_view() {
    // The banner draws the host name on every pane whatever is underneath it. A
    // name on screen the filter cannot find would be the filter lying about
    // what it searched.
    for mode in [
        Mode::Monitor,
        Mode::Graphs,
        Mode::Docker,
        Mode::Fetch,
        Mode::Upgrade,
    ] {
        let p = loaded_panel("web-01", mode);
        assert!(p.matches_filter("web-01"), "{mode:?} lost the host name");
        assert!(p.matches_filter("deploy"), "{mode:?} lost the user");
    }
}

#[test]
fn an_empty_query_matches_everything() {
    let p = loaded_panel("web-01", Mode::Monitor);
    assert!(p.matches_filter(""));
    assert!(p.matches_filter("   "));
}

#[test]
fn matching_ignores_case_and_surrounding_space() {
    let p = loaded_panel("web-01", Mode::Monitor);
    assert!(p.matches_filter("WEB-01"));
    assert!(p.matches_filter("  PostGres  "));
}

// ------------------------------------------------------------- the stats view

#[test]
fn the_stats_view_searches_processes_and_the_reported_host() {
    let p = loaded_panel("web-01", Mode::Monitor);
    assert!(p.matches_filter("postgres"), "a process was not searchable");
    assert!(p.matches_filter("sshd"));
    // The name the agent reports, which is not always the configured one.
    assert!(p.matches_filter("reported-name"));
    assert!(
        !p.matches_filter("mysql"),
        "it matched a process not running"
    );
}

#[test]
fn the_graphs_view_searches_the_same_stream_the_stats_view_does() {
    // `G` draws the Monitor stream as history. If it searched something else,
    // pressing `G` would change what `/` finds.
    let p = loaded_panel("web-01", Mode::Graphs);
    assert!(p.matches_filter("postgres"));
    assert!(!p.matches_filter("billing-api"));
}

// ------------------------------------------------------------ the docker view

#[test]
fn the_docker_view_searches_container_names_images_and_status() {
    let p = loaded_panel("web-01", Mode::Docker);
    assert!(
        p.matches_filter("billing-api"),
        "a container name was missed"
    );
    assert!(p.matches_filter("redis:7-alpine"), "an image was missed");
    assert!(
        p.matches_filter("registry.example.com"),
        "an image registry was missed"
    );
    assert!(p.matches_filter("Up 3 hours"), "the status text was missed");
    assert!(!p.matches_filter("mongo"));
}

// ------------------------------------------------------------- the fetch view

#[test]
fn the_fetch_view_searches_what_its_card_prints() {
    let p = loaded_panel("web-01", Mode::Fetch);
    assert!(p.matches_filter("ubuntu"));
    assert!(p.matches_filter("poweredge"));
    assert!(p.matches_filter("epyc"));
    assert!(p.matches_filter("6.8.0"), "the kernel was missed");
    assert!(!p.matches_filter("debian"));
}

// ----------------------------------------------------------- the upgrade view

#[test]
fn the_upgrade_view_searches_the_log() {
    let p = loaded_panel("web-01", Mode::Upgrade);
    assert!(p.matches_filter("libssl3"), "the log was not searchable");
    assert!(!p.matches_filter("libcurl"));
}

// -------------------------------------------------------------- the scoping

#[test]
fn a_view_does_not_search_what_another_view_is_holding() {
    // The whole point. Every payload is cached on this panel; only the one
    // being drawn answers.
    let stats = loaded_panel("web-01", Mode::Monitor);
    assert!(
        !stats.matches_filter("billing-api"),
        "the stats view searched a Docker payload nobody is looking at"
    );
    assert!(!stats.matches_filter("ubuntu"), "and a Fetch payload too");
    assert!(!stats.matches_filter("libssl3"), "and the upgrade log");

    let containers = loaded_panel("web-01", Mode::Docker);
    assert!(
        !containers.matches_filter("postgres"),
        "the Docker view searched the process table"
    );

    let card = loaded_panel("web-01", Mode::Fetch);
    assert!(!card.matches_filter("postgres"));
    assert!(!card.matches_filter("redis"));
}

#[test]
fn a_view_with_nothing_cached_yet_matches_only_the_host_and_user() {
    // A panel that has not received its first packet cannot match content it
    // has never been sent. It must not panic and must not match everything.
    let mut p = Panel::new(test_server("web-01"));
    for mode in [Mode::Monitor, Mode::Docker, Mode::Fetch, Mode::Upgrade] {
        p.mode = mode;
        assert!(p.matches_filter("web-01"), "{mode:?}");
        assert!(!p.matches_filter("postgres"), "{mode:?} matched thin air");
    }
}

// -------------------------------------------------------------- through the app

fn press(app: &mut App, code: KeyCode, tx: &mpsc::Sender<Msg>, tasks: &mut Tasks) {
    let (dims_tx, dims_rx) = watch::channel((80u16, 24u16));
    std::mem::forget(dims_tx);
    handle_key(
        KeyEvent::new_with_kind(code, KeyModifiers::NONE, KeyEventKind::Press),
        app,
        (80, 24),
        Arc::new(dims_rx),
        tx,
        tasks,
    );
}

#[tokio::test]
async fn typing_a_process_name_hides_the_hosts_not_running_it() {
    let _g = isolate().await;
    let mut app = App::new(vec![
        test_server("web-01"),
        test_server("web-02"),
        test_server("db-01"),
    ]);
    let epoch = app.panels_epoch;
    for (i, procs) in [
        vec!["nginx", "sshd"],
        vec!["nginx", "sshd"],
        vec!["postgres", "sshd"],
    ]
    .into_iter()
    .enumerate()
    {
        let gen = app.panels[i].gen;
        app.apply(Msg::Packet {
            panel: i,
            gen,
            epoch,
            payload: monitor("agent-host", &procs),
            dims: (80, 12),
        });
    }

    let (tx, _rx) = mpsc::channel::<Msg>(16);
    let mut tasks = Tasks::new(3);
    press(&mut app, KeyCode::Char('/'), &tx, &mut tasks);
    for c in "postgres".chars() {
        press(&mut app, KeyCode::Char(c), &tx, &mut tasks);
    }

    assert_eq!(
        app.filtered_indices(),
        vec![2],
        "the filter did not narrow to the host running postgres"
    );
    assert_eq!(app.visible_panes(), 1);
}

#[tokio::test]
async fn the_same_query_narrows_differently_once_the_view_changes() {
    // Documented, not accidental: `/` follows the view. A query that matched a
    // container stops matching when the user is no longer looking at the
    // containers.
    let _g = isolate().await;
    let mut app = App::new(vec![test_server("web-01"), test_server("db-01")]);
    let epoch = app.panels_epoch;
    for i in 0..2 {
        let gen = app.panels[i].gen;
        app.apply(Msg::Packet {
            panel: i,
            gen,
            epoch,
            payload: monitor(
                "agent-host",
                if i == 0 { &["nginx"] } else { &["postgres"] },
            ),
            dims: (80, 12),
        });
    }
    app.panels[0].last_docker = Some(docker(&[("billing-api", "team/billing:2.1")]));
    app.panels[1].last_docker = Some(docker(&[("redis", "redis:7-alpine")]));

    app.filter_query = "billing".to_string();
    assert!(
        app.filtered_indices().is_empty(),
        "the stats view found a container"
    );

    for p in &mut app.panels {
        p.mode = Mode::Docker;
    }
    assert_eq!(
        app.filtered_indices(),
        vec![0],
        "the Docker view did not find the container"
    );
}
