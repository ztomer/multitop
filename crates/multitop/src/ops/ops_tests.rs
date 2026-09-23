use std::time::Duration;

use serde_json::{json, Value};

use super::render::{age, render};
use super::*;
use crate::mcp::client::{Client, PROTOCOL_VERSION};
use crate::mcp::fake::{discover, serve, tools, Reply};

const NOW: i64 = 1_790_170_000;

/// The answers as .33 gave them (2026-09-23), trimmed.
fn health_answer() -> Value {
    json!({"checks": [
        {"check": "native:check_disk_space", "ok": true, "rc": 0, "ms": 128, "ts": NOW - 600},
        {"check": "native:check_vpn_leak", "ok": false, "rc": 1, "ms": 24003, "ts": NOW - 900},
    ]})
}
fn cron_answer() -> Value {
    json!({"jobs": [
        {"label": "backup", "schedule": "30 4 * * *", "last_start": NOW - 7200, "last_rc": 1, "last_ms": 9, "running_since": null, "alert_since": NOW - 7200},
        {"label": "healthcheck", "schedule": "*/30 * * * *", "last_start": NOW - 60, "last_rc": 0, "last_ms": 25392, "running_since": NOW - 30, "alert_since": null},
        {"label": "refresh_youtube_cookies", "schedule": "30 3 * * 0", "last_start": null, "last_rc": null, "last_ms": null, "running_since": null, "alert_since": null},
    ]})
}
fn containers_answer() -> Value {
    json!({"containers": [
        {"name": "gluetun", "id": "1", "image": "g", "state": "running", "status": "Up 1 hour (unhealthy)", "health": "unhealthy", "compose_project": "media_server", "compose_service": "vpn"},
        {"name": "calibre", "id": "2", "image": "c", "state": "running", "status": "Up 1 hour", "health": "none", "compose_project": "media_server", "compose_service": "calibre"},
        {"name": "great_sutherland", "id": "3", "image": "a", "state": "exited", "status": "Exited (0) 2 hours ago", "health": "none", "compose_project": null, "compose_service": null},
        {"name": "wger-web", "id": "4", "image": "w", "state": "exited", "status": "Exited (137) 5 minutes ago", "health": "none", "compose_project": "media_server", "compose_service": "wger-web"},
    ]})
}
fn alerts_answer() -> Value {
    json!({"events": [
        {"ts": format!("{}", NOW - 3600), "iso": "x", "subject": "[media_server] healthcheck FAILED (rc=1)", "body": "b"},
        {"ts": NOW - 60, "iso": "y", "subject": "[media_server] backup FAILED (rc=1)", "body": "b"},
    ], "marker": NOW})
}

#[test]
fn the_fleet_answers_parse_and_trouble_follows_the_follower_rule() {
    let h = parse_health(&health_answer()).unwrap();
    assert_eq!(
        (h[1].name.as_str(), h[1].ok, h[1].rc),
        ("native:check_vpn_leak", false, 1)
    );
    let j = parse_cron(&cron_answer()).unwrap();
    assert!(j[0].in_trouble() && !j[1].in_trouble() && !j[2].in_trouble());
    let c = parse_containers(&containers_answer()).unwrap();
    let trouble: Vec<&str> = c
        .iter()
        .filter(|c| c.in_trouble())
        .map(|c| c.name.as_str())
        .collect();
    // Unhealthy and a stopped compose service; never a finished one-shot.
    assert_eq!(trouble, ["gluetun", "wger-web"]);
    let a = parse_alerts(&alerts_answer()).unwrap();
    assert_eq!(a[0].ts, NOW - 3600, "a string ts parses");
    assert_eq!(
        parse_health(&json!({})).unwrap_err(),
        "the answer has no `checks` list"
    );
    assert!(
        parse_cron(&json!({})).is_err()
            && parse_containers(&json!([])).is_err()
            && parse_alerts(&json!(1)).is_err()
    );
}

fn host(
    tool_names: &'static [&'static str],
    docker: Reply,
) -> impl Fn(&str, &Value) -> Reply + Send + 'static {
    move |method, params| match method {
        "server/discover" => Reply::Result(discover(&[PROTOCOL_VERSION])),
        "tools/list" => Reply::Result(tools(tool_names)),
        "resources/read" => {
            Reply::Result(json!({"contents": [{"text": health_answer().to_string()}]}))
        }
        "tools/call" => match params["name"].as_str().unwrap() {
            "cron.status" => Reply::Result(json!({"structuredContent": cron_answer()})),
            "docker.containers" => docker.clone(),
            "alerts.since" => {
                assert_eq!(params["arguments"]["marker"], NOW - ALERT_WINDOW_S);
                Reply::Result(json!({"structuredContent": alerts_answer()}))
            }
            _ => Reply::Error(-32602, "unknown"),
        },
        _ => Reply::Error(-32601, "no"),
    }
}

#[tokio::test]
async fn gather_asks_only_what_the_host_lists_and_one_failure_hides_nothing() {
    // .158: no Docker tool at all - an absence, stated.
    let (rd, wr, seen) = serve(host(
        &["health.run", "cron.status", "alerts.since"],
        Reply::Silent,
    ));
    let mut client = Client::connect(rd, wr, Duration::from_secs(2))
        .await
        .unwrap();
    let s = gather(&mut client, NOW).await;
    assert_eq!(s.containers, Part::Absent("no Docker on this host".into()));
    assert!(matches!(s.health, Part::Ready(ref v) if v.len() == 2));
    assert!(matches!(s.alerts, Part::Ready(ref v) if v.len() == 2));
    assert!(!seen
        .lock()
        .unwrap()
        .iter()
        .any(|r| r["params"]["name"] == "docker.containers"));
    assert_eq!(s.server, ("mcp_host".into(), "1.0.0".into()));
    // A host whose docker.containers fails: that section says so, the rest stand.
    let (rd, wr, _) = serve(host(
        &[
            "health.run",
            "cron.status",
            "docker.containers",
            "alerts.since",
        ],
        Reply::Error(-32603, "docker is not answering"),
    ));
    let mut client = Client::connect(rd, wr, Duration::from_secs(2))
        .await
        .unwrap();
    let s = gather(&mut client, NOW).await;
    assert_eq!(
        s.containers,
        Part::Failed("docker is not answering (-32603)".into())
    );
    assert!(matches!(s.cron, Part::Ready(_)));
    // A host listing none of the four.
    let (rd, wr, _) = serve(host(&[], Reply::Silent));
    let mut client = Client::connect(rd, wr, Duration::from_secs(2))
        .await
        .unwrap();
    let s = gather(&mut client, NOW).await;
    assert!(
        matches!(s.health, Part::Absent(_))
            && matches!(s.cron, Part::Absent(_))
            && matches!(s.alerts, Part::Absent(_))
    );
}

fn snapshot() -> Snapshot {
    Snapshot {
        server: ("mcp_host".into(), "1.65.2".into()),
        at: NOW - 12,
        health: Part::Ready(parse_health(&health_answer()).unwrap()),
        cron: Part::Ready(parse_cron(&cron_answer()).unwrap()),
        containers: Part::Ready(parse_containers(&containers_answer()).unwrap()),
        alerts: Part::Ready(parse_alerts(&alerts_answer()).unwrap()),
    }
}

/// The frame with its escape sequences removed.
fn plain(lines: &[String]) -> Vec<String> {
    let esc = regex::Regex::new("\x1b\\[[0-9;]*m").unwrap();
    lines
        .iter()
        .map(|l| esc.replace_all(l, "").into_owned())
        .collect()
}

const PAL: &multitop_agent::color::Palette = &multitop_agent::color::KARE;

#[test]
fn a_troubled_host_names_each_thing_in_trouble_under_its_section() {
    let f = plain(&render(
        "media-server",
        &OpsState::Ready(Box::new(snapshot())),
        80,
        40,
        NOW,
        PAL,
    ));
    assert!(
        f[0].contains("ｍｅｄｉａ"),
        "row 0 is the host banner (fullwidth): {f:?}"
    );
    let body = &f[1..];
    let want = [
        "mcp_host 1.65.2 · asked 12s ago",
        "✗ health  1 of 2 failing",
        "    check_vpn_leak  rc 1, 15m ago",
        "✗ cron    1 of 3 failing, running: healthcheck",
        "    backup  rc 1, 2h ago, alerting 2h",
        "✗ docker  2 of 4 in trouble",
        "    gluetun  unhealthy",
        "    wger-web  Exited (137) 5 minutes ago",
        "⚠ alerts  2 in 24 h",
        "    1m ago  [media_server] backup FAILED (rc=1)",
        "    1h ago  [media_server] healthcheck FAILED (rc=1)",
    ];
    assert_eq!(body, want);
}

#[test]
fn a_clean_host_is_four_lines_and_absences_are_stated_not_alarmed() {
    let mut s = snapshot();
    s.health = Part::Ready(vec![Check {
        name: "a".into(),
        ok: true,
        rc: 0,
        ts: NOW,
    }]);
    s.cron = Part::Ready(vec![]);
    s.containers = Part::Absent("no Docker on this host".into());
    s.alerts = Part::Ready(vec![]);
    let f = plain(&render(
        "pihole",
        &OpsState::Ready(Box::new(s)),
        80,
        40,
        NOW,
        PAL,
    ));
    assert_eq!(
        &f[2..],
        [
            "✓ health  1 check ok",
            "✓ cron    0 jobs ok",
            "· docker  no Docker on this host",
            "✓ alerts  none in 24 h"
        ]
    );
    let mut s = snapshot();
    s.cron = Part::Failed("tools/call: no answer in 10s".into());
    let f = plain(&render(
        "h",
        &OpsState::Ready(Box::new(s)),
        80,
        40,
        NOW,
        PAL,
    ));
    assert!(
        f.contains(&"⚠ cron    could not read: tools/call: no answer in 10s".to_string()),
        "{f:?}"
    );
}

#[test]
fn every_state_says_what_it_is() {
    let f = plain(&render("h", &OpsState::NotConfigured, 80, 10, NOW, PAL));
    assert_eq!(f[1], "· ops     no mcp_host command for this host");
    assert!(
        f[2].contains("mcp = \""),
        "the action sits beside the status: {f:?}"
    );
    assert_eq!(
        plain(&render("h", &OpsState::Asking, 80, 10, NOW, PAL))[1],
        "→ asking mcp_host..."
    );
    let failed = OpsState::Failed {
        why: "ssh: connect to host h port 22: Connection refused".into(),
        last: None,
    };
    let f = plain(&render("h", &failed, 80, 10, NOW, PAL));
    assert_eq!(
        f,
        [
            f[0].clone(),
            "⚠ mcp     ssh: connect to host h port 22: Connection refused".to_string()
        ]
    );
    let stale = OpsState::Failed {
        why: "gone".into(),
        last: Some(Box::new(snapshot())),
    };
    let f = plain(&render("h", &stale, 80, 40, NOW, PAL));
    assert_eq!(f[2], "  the last answer, 12s old:");
    assert_eq!(f[3], "✗ health  1 of 2 failing");
}

#[test]
fn the_frame_fits_the_pane() {
    let f = render("h", &OpsState::Ready(Box::new(snapshot())), 30, 6, NOW, PAL);
    assert_eq!(f.len(), 6);
    let p = plain(&f);
    assert_eq!(p[5], "  +7 more");
    assert!(p[1..].iter().all(|l| l.chars().count() <= 30), "{p:?}");
}

#[test]
fn the_glyphs_alone_tell_the_tones_apart() {
    // Never one channel: with colour gone, each tone is still its own glyph.
    let glyphs = ["✓", "✗", "⚠", "·", "→"];
    let set: std::collections::BTreeSet<&str> = glyphs.iter().copied().collect();
    assert_eq!(set.len(), glyphs.len());
    assert_eq!(
        (
            age(NOW, NOW),
            age(NOW - 61, NOW),
            age(NOW - 7300, NOW),
            age(NOW - 200_000, NOW)
        ),
        ("0s".into(), "1m".into(), "2h".into(), "2d".into())
    );
    assert_eq!(
        age(NOW + 5, NOW),
        "0s",
        "a clock ahead of ours is not negative time"
    );
}
