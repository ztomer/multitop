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
        {"instance": "media_server", "check": "native:check_disk_space", "ok": true, "rc": 0, "ms": 128, "ts": NOW - 600, "age_s": 600, "age": "10 min ago"},
        {"instance": "media_server", "check": "native:check_vpn_leak", "ok": false, "rc": 1, "ms": 24003, "ts": NOW - 900, "age_s": 900, "age": "15 min ago"},
    ], "oldest_age_s": 900, "oldest_age": "15 min ago",
    "cadence_s": 900, "cadence": "every 15 min"})
}

/// The machine's suite: the two checks above are in it.
fn suite_answer() -> Value {
    json!({"checks": [
        {"instance": "media_server", "name": "native:check_disk_space", "schedule": "*/15 * * * *", "in_suite": true},
        {"instance": "media_server", "name": "native:check_vpn_leak", "schedule": "*/15 * * * *", "in_suite": true},
        {"instance": "media_server", "name": "native:check_image_drift_sweep", "schedule": "35 4 * * *", "in_suite": false},
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
        (h.checks[1].name.as_str(), h.checks[1].ok, h.checks[1].rc),
        ("native:check_vpn_leak", false, 1)
    );
    assert_eq!(h.oldest_age.as_deref(), Some("15 min ago"));
    assert_eq!(h.oldest_age_s, Some(900));
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

pub(super) fn host(
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
            "health.list" => Reply::Result(json!({"structuredContent": suite_answer()})),
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
        &["health.list", "cron.status", "alerts.since"],
        Reply::Silent,
    ));
    let mut client = Client::connect(rd, wr, Duration::from_secs(2))
        .await
        .unwrap();
    let s = gather(&mut client, NOW, Previous::default()).await;
    assert_eq!(s.containers, Part::Absent("no Docker on this host".into()));
    assert!(matches!(s.health, Part::Ready(ref h) if h.checks.len() == 2));
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
            "health.list",
            "cron.status",
            "docker.containers",
            "alerts.since",
        ],
        Reply::Error(-32603, "docker is not answering"),
    ));
    let mut client = Client::connect(rd, wr, Duration::from_secs(2))
        .await
        .unwrap();
    let s = gather(&mut client, NOW, Previous::default()).await;
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
    let s = gather(&mut client, NOW, Previous::default()).await;
    assert!(
        matches!(s.health, Part::Absent(_))
            && matches!(s.cron, Part::Absent(_))
            && matches!(s.alerts, Part::Absent(_))
    );
}

pub(super) fn snapshot() -> Snapshot {
    Snapshot {
        server: ("mcp_host".into(), "1.65.2".into()),
        at: NOW - 12,
        sampled_every_s: FALLBACK_INTERVAL_S,
        catalogue: Catalogue::Listed { ttl_s: 3_600 },
        health: Part::Ready(parse_health(&health_answer()).unwrap()),
        vpn: Part::Absent("this host lists no net.egress".into()),
        cron: Part::Ready(parse_cron(&cron_answer()).unwrap()),
        containers: Part::Ready(parse_containers(&containers_answer()).unwrap()),
        zfs: Part::Absent("this host lists no zfs.health".into()),
        disk: Part::Absent("this host lists no disk.usage".into()),
        facts: Part::Absent("this host lists no host.facts".into()),
        alerts: Part::Ready(parse_alerts(&alerts_answer()).unwrap()),
    }
}

/// The frame with its escape sequences removed.
pub(super) fn plain(lines: &[String]) -> Vec<String> {
    let esc = regex::Regex::new("\x1b\\[[0-9;]*m").unwrap();
    lines
        .iter()
        .map(|l| esc.replace_all(l, "").into_owned())
        .collect()
}

/// The palette these tests render with: a view over Kare's own stated
/// background, so the escapes are Kare's own. Every assertion below that names a
/// colour is therefore asserting what the theme says, and none of them has to
/// know that a view exists.
///
/// A function rather than a `const`, because a view OWNS its escapes and a
/// `const` cannot allocate. Each call builds a fresh one, which costs a dozen
/// small allocations per test rather than nothing — and nothing would mean
/// testing a `Palette`, which is not what production renders with.
pub(super) fn pal() -> multitop_agent::palette_view::PaletteView {
    multitop_agent::palette_view::PaletteView::for_theme(&multitop_agent::color::KARE)
}
#[test]
fn a_troubled_host_names_each_thing_in_trouble_under_its_section() {
    let f = plain(&render(
        "media-server",
        &OpsState::Ready(Box::new(snapshot())),
        80,
        40,
        NOW,
        &pal(),
    ));
    assert!(
        f[0].contains("ｍｅｄｉａ"),
        "row 0 is the host banner (fullwidth): {f:?}"
    );
    let body = &f[1..];
    let want = [
        "mcp_host 1.65.2 · asked 12s ago",
        "tool list: from the server (window 1h)",
        "· no cpu or memory: this host lists no host.facts",
        "✗ health     1 of 2 failing · 15 min ago",
        "    check_vpn_leak  rc 1, 15 min ago",
        "· vpn        this host lists no net.egress",
        "✗ cron       1 of 3 failing, running: healthcheck",
        "    backup  rc 1, 2h ago, alerting 2h",
        "✗ containers media_server 2 of 3 in trouble",
        "    gluetun  unhealthy",
        "    wger-web  Exited (137) 5 minutes ago",
        "· containers none running, 1 stopped",
        "· zfs        this host lists no zfs.health",
        "· disk       this host lists no disk.usage",
        "⚠ alerts     2 in 24 h",
        "    1m ago  [media_server] backup FAILED (rc=1)",
        "    1h ago  [media_server] healthcheck FAILED (rc=1)",
    ];
    assert_eq!(body, want);
}

#[test]
fn a_clean_host_is_five_lines_and_absences_are_stated_not_alarmed() {
    let mut s = snapshot();
    s.health = Part::Ready(Health {
        checks: vec![Check {
            name: "a".into(),
            instance: Some("media_server".into()),
            ok: true,
            rc: 0,
            ts: NOW,
            age: Some("13 min ago".into()),
        }],
        oldest_age: Some("13 min ago".into()),
        oldest_age_s: Some(780),
        cadence: Cadence {
            secs: Some(900),
            words: Some("every 15 min".into()),
        },
    });
    s.cron = Part::Ready(vec![]);
    s.containers = Part::Absent("no Docker on this host".into());
    s.alerts = Part::Ready(vec![]);
    let f = plain(&render(
        "pihole",
        &OpsState::Ready(Box::new(s)),
        80,
        40,
        NOW,
        &pal(),
    ));
    assert_eq!(
        &f[3..],
        [
            "· no cpu or memory: this host lists no host.facts",
            "✓ health     1 check ok · 13 min ago",
            "· vpn        this host lists no net.egress",
            "· cron       no jobs",
            "· containers no Docker on this host",
            "· zfs        this host lists no zfs.health",
            "· disk       this host lists no disk.usage",
            "✓ alerts     none in 24 h"
        ]
    );
    // **A SECTION THAT FAILED AND A SECTION THAT IS UNKNOWN ARE TWO
    // DIFFERENT THINGS.** The first is the estate's own report; the
    // second is a fact this client does not have, because the answer did
    // not arrive (servers ROADMAP E74).
    let mut s = snapshot();
    s.cron = Part::Failed("cron.status: no such unit (-32602)".into());
    s.containers = Part::Unknown("docker.containers: no answer in 20s".into());
    let f = plain(&render(
        "h",
        &OpsState::Ready(Box::new(s)),
        80,
        40,
        NOW,
        &pal(),
    ));
    assert!(
        f.contains(&"⚠ cron       failed: cron.status: no such unit (-32602)".to_string()),
        "{f:?}"
    );
    assert!(
        f.contains(&"⚠ containers unknown: docker.containers: no answer in 20s".to_string()),
        "{f:?}"
    );
}

#[test]
fn every_state_says_what_it_is() {
    let f = plain(&render("h", &OpsState::NotConfigured, 80, 10, NOW, &pal()));
    assert_eq!(f[1], "· ops        no mcp_host command for this host");
    assert!(
        f[2].contains("mcp = \""),
        "the action sits beside the status: {f:?}"
    );
    assert_eq!(
        plain(&render("h", &OpsState::Asking, 80, 10, NOW, &pal()))[1],
        "→ asking mcp_host..."
    );
    let failed = OpsState::Failed {
        why: "ssh: connect to host h port 22: Connection refused".into(),
        last: None,
    };
    let f = plain(&render("h", &failed, 80, 10, NOW, &pal()));
    assert_eq!(
        f,
        [
            f[0].clone(),
            "⚠ mcp        ssh: connect to host h port 22: Connection refused".to_string()
        ]
    );
    let stale = OpsState::Failed {
        why: "gone".into(),
        last: Some(Box::new(snapshot())),
    };
    let f = plain(&render("h", &stale, 80, 40, NOW, &pal()));
    assert_eq!(f[2], "  the last answer, 12s old:");
    assert_eq!(f[4], "✗ health     1 of 2 failing · 15 min ago");
}

#[test]
fn the_frame_fits_the_pane() {
    let f = render(
        "h",
        &OpsState::Ready(Box::new(snapshot())),
        30,
        6,
        NOW,
        &pal(),
    );
    assert_eq!(f.len(), 6);
    let p = plain(&f);
    assert_eq!(p[5], "  +13 more");
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

#[test]
fn a_container_without_a_health_check_is_not_counted_as_unhealthy() {
    // .33, 2026-09-23: "44 running, 33 healthy" read as 11 unhealthy; none
    // were - 11 define no Docker health check, which is a different fact.
    let mut s = snapshot();
    s.containers = Part::Ready(
        parse_containers(&json!({"containers": [
            {"name": "a", "id": "1", "image": "i", "state": "running", "status": "Up", "health": "healthy", "compose_project": "p", "compose_service": "a"},
            {"name": "b", "id": "2", "image": "i", "state": "running", "status": "Up", "health": "healthy", "compose_project": "p", "compose_service": "b"},
            {"name": "c", "id": "3", "image": "i", "state": "running", "status": "Up", "health": "none", "compose_project": "p", "compose_service": "c"},
        ]}))
        .unwrap(),
    );
    let f = plain(&render(
        "h",
        &OpsState::Ready(Box::new(s)),
        80,
        40,
        NOW,
        &pal(),
    ));
    assert!(
        f.contains(&"⚠ containers p 3 running · 1 unchecked".to_string()),
        "{f:?}"
    );
    // Every one checked and healthy: said in three words.
    let mut s = snapshot();
    s.containers = Part::Ready(
        parse_containers(&json!({"containers": [
            {"name": "a", "id": "1", "image": "i", "state": "running", "status": "Up", "health": "healthy", "compose_project": "p", "compose_service": "a"},
        ]}))
        .unwrap(),
    );
    let f = plain(&render(
        "h",
        &OpsState::Ready(Box::new(s)),
        80,
        40,
        NOW,
        &pal(),
    ));
    assert!(f.contains(&"✓ containers p 1 running".to_string()), "{f:?}");
}

#[test]
fn an_empty_section_says_so_and_never_claims_all_is_well() {
    // An emptied Docker host read "✓ docker  0 running, all healthy"
    // (2026-10-10): a green verdict over nothing. Every section, empty.
    let mut s = snapshot();
    s.health = Part::Ready(Health {
        checks: Vec::new(),
        oldest_age: Some("with no timestamp".into()),
        oldest_age_s: None,
        cadence: Cadence {
            secs: Some(900),
            words: Some("every 15 min".into()),
        },
    });
    s.cron = Part::Ready(Vec::new());
    s.containers = Part::Ready(Vec::new());
    let f = plain(&render(
        "h",
        &OpsState::Ready(Box::new(s)),
        80,
        40,
        NOW,
        &pal(),
    ));
    for line in [
        "⚠ health     no verdict published: nothing has run on this machine",
        "· cron       no jobs",
        "· containers no containers",
    ] {
        assert!(f.contains(&line.to_string()), "{line} missing: {f:?}");
    }
    for label in ["health", "cron", "containers"] {
        assert!(
            !f.iter().any(|l| l.starts_with(&format!("✓ {label}"))),
            "{label} is green over nothing: {f:?}"
        );
    }

    // Containers that all exited cleanly: none is running, so none is healthy.
    let mut s = snapshot();
    s.containers = Part::Ready(
        parse_containers(&json!({"containers": [
            {"name": "a", "state": "exited", "status": "Exited (0) 2 days ago", "health": ""},
            {"name": "b", "state": "exited", "status": "Exited (0) 3 days ago", "health": ""},
        ]}))
        .unwrap(),
    );
    let f = plain(&render(
        "h",
        &OpsState::Ready(Box::new(s)),
        80,
        40,
        NOW,
        &pal(),
    ));
    assert!(
        f.contains(&"· containers none running, 2 stopped".to_string()),
        "{f:?}"
    );
}
