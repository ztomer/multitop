//! The Ops view end to end (servers ROADMAP 12.17b): `p` through the real
//! key path, a real process, the stateless client, the poll, `App::apply`,
//! and the frame it draws.
//!
//! The server is the real one: this build's agent in its `mcp` mode, the
//! local panel's default source (roadmap N3). It used to be a Python
//! stand-in, which proved the client against a server nobody ships. Three
//! hosts: this machine with no `mcp` (the agent, by default), one whose `mcp`
//! command runs the agent in a pinned environment (so its answer is the
//! same on every machine), and a remote host with no `mcp` (not configured,
//! and never polled).
#![cfg(test)]

mod common;

use std::sync::Arc;
use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use multitop::app::{App, Msg};
use multitop::config::Server;
use multitop::panel::Mode;
use multitop::password_store;
use multitop::run::{handle_key, Tasks};
use tokio::sync::{mpsc, watch};

fn local(host: &str, mcp: Option<String>) -> Server {
    Server {
        user: "t".to_string(),
        mcp,
        ..common::local_server(host)
    }
}

fn press(c: char, app: &mut App, tx: &mpsc::Sender<Msg>, tasks: &mut Tasks) {
    let (_dims_tx, dims_rx) = watch::channel((80u16, 24u16));
    handle_key(
        KeyEvent::new_with_kind(KeyCode::Char(c), KeyModifiers::NONE, KeyEventKind::Press),
        app,
        (80, 24),
        &Arc::new(dims_rx),
        tx,
        tasks,
    );
}

fn plain(lines: &[String]) -> String {
    let esc = regex::Regex::new("\x1b\\[[0-9;]*m").unwrap();
    esc.replace_all(&lines.join("\n"), "").into_owned()
}

fn remote(host: &str) -> Server {
    Server {
        port: 22,
        ..local("unused", None)
    }
    .with_host(host)
}

trait WithHost {
    fn with_host(self, host: &str) -> Self;
}

impl WithHost for Server {
    fn with_host(mut self, host: &str) -> Self {
        self.host = host.to_string();
        self
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn p_polls_each_hosts_server_and_draws_what_it_says() {
    let _g = password_store::lock_for_test_async().await;
    password_store::enable_mock_store();
    let home = tempfile::tempdir().unwrap();
    // The agent with nothing to report: an empty home (no launch agents), and
    // a Docker daemon that is configured but not there -- a fault, which the
    // view must show as one.
    let pinned = format!(
        "HOME={} DOCKER_HOST=unix://{}/no.sock DOCKER_CONFIG={} {} --agent mcp",
        home.path().display(),
        home.path().display(),
        home.path().display(),
        env!("CARGO_BIN_EXE_multitop"),
    );
    let mut app = App::new(vec![
        local("127.0.0.1", None),
        local("pinned", Some(pinned)),
        remote("nas.invalid"),
        local(
            "broken",
            Some("echo 'bin/mcp_host: not found' >&2; exit 127".to_string()),
        ),
    ]);
    let (tx, mut rx) = mpsc::channel(16);
    let mut tasks = Tasks::new(4);

    press('p', &mut app, &tx, &mut tasks);
    assert!(app.panels.iter().all(|p| p.mode == Mode::Ops));
    // This machine is polled with no `mcp`; a remote host without one is not.
    let polled: Vec<bool> = tasks.ops.iter().map(Option::is_some).collect();
    assert_eq!(polled, [true, true, false, true]);
    assert!(
        plain(&app.panels[0].view).contains("asking"),
        "{}",
        plain(&app.panels[0].view)
    );
    assert!(
        plain(&app.panels[2].view).contains("· ops     no mcp_host command for this host"),
        "{}",
        plain(&app.panels[2].view)
    );

    let mut seen = [false, false, false, false];
    while !(seen[0] && seen[1] && seen[3]) {
        let msg = tokio::time::timeout(Duration::from_secs(20), rx.recv())
            .await
            .expect("an Ops answer within 20 s")
            .expect("the channel stays open");
        if let Msg::Ops { panel, .. } = &msg {
            seen[*panel] = true;
        }
        app.apply(msg);
    }
    let version = multitop_agent::consts::AGENT_VERSION;
    for i in [0, 1] {
        let view = plain(&app.panels[i].view);
        assert!(
            view.contains(&format!("multitop-agent {version} · asked")),
            "{view}"
        );
        // Absent WITH the server's reason, never a green "none".
        assert!(
            view.contains("· health  no health suite on this machine"),
            "{view}"
        );
        assert!(
            view.contains("· alerts  nothing on this machine logs alerts"),
            "{view}"
        );
    }
    let pinned = plain(&app.panels[1].view);
    assert!(
        pinned.contains("⚠ docker  could not read: docker.containers: docker at unix://"),
        "a configured daemon that is down is a fault: {pinned}"
    );
    if cfg!(target_os = "macos") {
        assert!(pinned.contains("✓ cron    0 jobs ok"), "{pinned}");
    } else {
        assert!(pinned.contains("· cron    no launchd here"), "{pinned}");
    }
    let broken = plain(&app.panels[3].view);
    assert!(
        broken.contains("⚠ mcp     mcp_host closed the session: bin/mcp_host: not found"),
        "{broken}"
    );

    // `/` searches what the view shows.
    app.filter_query = "not found".to_string();
    assert_eq!(app.filtered_indices(), vec![3]);
    app.filter_query.clear();

    // Leaving the view ends every poll (the event loop reconciles after each key).
    press('s', &mut app, &tx, &mut tasks);
    tasks.retire_ops(&app.panels);
    assert!(tasks.ops.iter().all(Option::is_none));
}
