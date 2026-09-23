//! The Ops view end to end (servers ROADMAP 12.17b): `p` through the real
//! key path, a real process started by `sh -c` for a local panel, the
//! stateless client, the poll, `App::apply`, and the frame it draws.
//!
//! The host's `mcp_host` is a small Python stand-in that answers the
//! protocol and refuses any request without the 2026-07-28 `_meta`.
#![cfg(test)]

use std::sync::Arc;
use std::time::Duration;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use multitop::app::{App, Msg};
use multitop::config::Server;
use multitop::panel::Mode;
use multitop::password_store;
use multitop::run::{handle_key, Tasks};
use tokio::sync::{mpsc, watch};

const FAKE_MCP_HOST: &str = r#"
import sys, json
for line in sys.stdin:
    r = json.loads(line)
    meta = r.get("params", {}).get("_meta", {})
    if meta.get("io.modelcontextprotocol/protocolVersion") != "2026-07-28":
        out = {"jsonrpc": "2.0", "id": r["id"], "error": {"code": -32600, "message": "no version"}}
    else:
        m = r["method"]
        if m == "server/discover":
            res = {"supportedVersions": ["2026-07-28"],
                   "_meta": {"io.modelcontextprotocol/serverInfo": {"name": "mcp_host", "version": "9.9.9"}}}
        elif m == "tools/list":
            res = {"tools": [{"name": "cron.status"}]}
        else:
            res = {"structuredContent": {"jobs": [
                {"label": "backup", "last_start": 1, "last_rc": 2, "running_since": None, "alert_since": None}]}}
        out = {"jsonrpc": "2.0", "id": r["id"], "result": res}
    print(json.dumps(out), flush=True)
"#;

fn local(host: &str, mcp: Option<String>) -> Server {
    Server {
        host: host.to_string(),
        port: 0,
        user: "t".to_string(),
        upgrade_cmd: None,
        custom_command: None,
        mcp,
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn p_polls_each_hosts_mcp_host_and_draws_what_it_says() {
    let _g = password_store::lock_for_test_async().await;
    password_store::enable_mock_store();
    let dir = tempfile::tempdir().unwrap();
    let script = dir.path().join("mcp_host.py");
    std::fs::write(&script, FAKE_MCP_HOST).unwrap();
    let mut app = App::new(vec![
        local("media", Some(format!("python3 {}", script.display()))),
        local(
            "broken",
            Some("echo 'bin/mcp_host: not found' >&2; exit 127".to_string()),
        ),
        local("nas", None),
    ]);
    let (tx, mut rx) = mpsc::channel(16);
    let mut tasks = Tasks::new(3);

    press('p', &mut app, &tx, &mut tasks);
    assert!(app.panels.iter().all(|p| p.mode == Mode::Ops));
    // A poll for each host with a command, none for the host without.
    assert!(tasks.ops[0].is_some() && tasks.ops[1].is_some() && tasks.ops[2].is_none());
    assert!(plain(&app.panels[0].view).contains("→ asking mcp_host..."));
    assert!(
        plain(&app.panels[2].view).contains("· ops     no mcp_host command for this host"),
        "{}",
        plain(&app.panels[2].view)
    );

    // One answer from each polled host.
    let mut seen = [false, false];
    while !(seen[0] && seen[1]) {
        let msg = tokio::time::timeout(Duration::from_secs(20), rx.recv())
            .await
            .expect("an Ops answer within 20 s")
            .expect("the channel stays open");
        if let Msg::Ops { panel, .. } = &msg {
            seen[*panel] = true;
        }
        app.apply(msg);
    }
    let media = plain(&app.panels[0].view);
    assert!(media.contains("mcp_host 9.9.9 · asked"), "{media}");
    assert!(
        media.contains("· health  this host lists no health checks"),
        "{media}"
    );
    assert!(media.contains("✗ cron    1 of 1 failing"), "{media}");
    assert!(media.contains("    backup  rc 2"), "{media}");
    assert!(
        media.contains("· docker  no Docker on this host"),
        "{media}"
    );
    let broken = plain(&app.panels[1].view);
    assert!(
        broken.contains("⚠ mcp     mcp_host closed the session: bin/mcp_host: not found"),
        "{broken}"
    );

    // `/` searches what the view shows.
    app.filter_query = "backup".to_string();
    assert_eq!(app.filtered_indices(), vec![0]);
    app.filter_query = "not found".to_string();
    assert_eq!(app.filtered_indices(), vec![1]);
    app.filter_query.clear();

    // Leaving the view ends every poll (the event loop reconciles after each key).
    press('s', &mut app, &tx, &mut tasks);
    tasks.retire_ops(&app.panels);
    assert!(tasks.ops.iter().all(Option::is_none));
}
