//! The health section reads the PUBLISHED verdict, and says how old it is.
//!
//! servers ROADMAP E12: the clients used to re-run the server's own
//! 50-check suite to get a verdict the server publishes every 15 minutes.
//! The swap is only honest if the AGE comes with it - a verdict that
//! cannot be dated is a claim nobody can check, and one that is presented
//! as current when it is 29 minutes old is a lie.
//!
//! Four things are proved here, and each of them is a way the swap could
//! have gone wrong:
//!
//! - the age is ON the line, in the server's own words;
//! - a verdict past the estate's interval says so, and is not green;
//! - a machine that published no verdict says so, and is never an
//!   all-clear;
//! - the verdict is the machine's PERIODIC SUITE - the published table
//!   also carries the daily checks, whose verdicts are a day old by
//!   design, and pairing those with the suite's freshness would be a
//!   number and an age that do not describe the same set;
//! - and the Ops view never runs the suite: it reads.

use serde_json::json;

use super::answers::{parse_health, parse_suite};
use super::model::{Check, Health};
use super::render::render;
use super::tests::{pal, plain};
use super::*;
use crate::mcp::client::{Client, PROTOCOL_VERSION};
use crate::mcp::fake::{discover, serve, tools, Reply};

const NOW: i64 = 1_790_170_000;

/// The published verdict, as the estate's server publishes it: every
/// check's last result, and the machine's own freshness.
fn published() -> serde_json::Value {
    json!({"checks": [
        {"instance": "media_server", "check": "native:check_disk_space", "ok": true, "rc": 0, "ms": 128, "ts": NOW - 600, "age_s": 600, "age": "10 min ago"},
        {"instance": "media_server", "check": "native:check_vpn_leak", "ok": true, "rc": 0, "ms": 24003, "ts": NOW - 600, "age_s": 600, "age": "10 min ago"},
        {"instance": "media_server", "check": "native:check_image_drift_sweep", "ok": true, "rc": 0, "ms": 1991, "ts": NOW - 7200, "age_s": 7200, "age": "2 h ago"},
    ], "oldest_age_s": 600, "oldest_age": "10 min ago"})
}

/// The machine's suite: the two periodic checks are in it, the daily one
/// is not.
fn suite() -> serde_json::Value {
    json!({"checks": [
        {"instance": "media_server", "name": "native:check_disk_space", "schedule": "*/15 * * * *", "in_suite": true},
        {"instance": "media_server", "name": "native:check_vpn_leak", "schedule": "*/15 * * * *", "in_suite": true},
        {"instance": "media_server", "name": "native:check_image_drift_sweep", "schedule": "35 4 * * *", "in_suite": false},
    ]})
}

/// The health section's line, without the escape sequences.
fn health_line(h: &Health) -> String {
    let mut s = snapshot(h);
    s.cron = Part::Ready(Vec::new());
    s.containers = Part::Absent("no Docker on this host".into());
    s.alerts = Part::Ready(Vec::new());
    let f = plain(&render(
        "media-server",
        &OpsState::Ready(Box::new(s)),
        80,
        40,
        NOW,
        &pal(),
    ));
    f.into_iter()
        .find(|l| l.contains("health"))
        .unwrap_or_default()
}

/// A snapshot whose only interesting section is the health one.
fn snapshot(h: &Health) -> Snapshot {
    Snapshot {
        server: ("mcp_host".into(), "1.99.11".into()),
        at: NOW - 12,
        health: Part::Ready(h.clone()),
        vpn: Part::Absent("this host lists no net.egress".into()),
        cron: Part::Ready(Vec::new()),
        containers: Part::Ready(Vec::new()),
        zfs: Part::Absent("this host lists no zfs.health".into()),
        disk: Part::Absent("this host lists no disk.usage".into()),
        facts: Part::Absent("this host lists no host.facts".into()),
        alerts: Part::Ready(Vec::new()),
    }
}

/// A verdict, narrowed to the suite, as the client reads it.
fn verdict() -> Health {
    let mut h = parse_health(&published()).unwrap();
    h.narrow_to(&parse_suite(&suite()).unwrap());
    h
}

/// The age is ON the line: a verdict that cannot be dated is a claim
/// nobody can check, and the server's own words date it.
#[test]
fn the_health_line_carries_the_verdicts_age() {
    let line = health_line(&verdict());
    assert!(
        line.contains("2 checks ok · 10 min ago"),
        "the age is not on the line: {line}"
    );
}

/// A verdict past the estate's interval is not a current answer: the
/// estate's own cron did not fire, and the line says so and is not green.
#[test]
fn a_verdict_past_the_interval_is_stale_and_says_so() {
    let mut h = verdict();
    h.oldest_age = Some("29 min ago".into());
    h.oldest_age_s = Some(29 * 60);
    let line = health_line(&h);
    assert!(
        line.contains("29 min ago · stale"),
        "a stale verdict does not say it is stale: {line}"
    );
    assert!(
        !line.starts_with('✓'),
        "a stale verdict is not green: {line}"
    );
}

/// A machine that published no verdict says so. It is never an all-clear:
/// "could not ask is never fine".
#[test]
fn a_machine_that_published_no_verdict_says_so_and_is_not_green() {
    let h = Health {
        checks: Vec::new(),
        oldest_age: Some("with no timestamp".into()),
        oldest_age_s: None,
    };
    let line = health_line(&h);
    assert!(
        line.contains("no verdict published"),
        "an absent verdict is not stated: {line}"
    );
    assert!(!line.starts_with('✓'), "an absent verdict is green: {line}");
}

/// The verdict is the machine's PERIODIC SUITE. The published table also
/// carries the daily checks - whose verdicts are a day old by design - and
/// a count that included them would be paired with an age that does not
/// describe them.
#[test]
fn the_verdict_is_the_suites_and_not_the_whole_table() {
    let h = verdict();
    assert_eq!(h.checks.len(), 2, "the daily check is not in the suite");
    assert!(
        !h.checks
            .iter()
            .any(|c| c.name.ends_with("check_image_drift_sweep")),
        "the daily check is not in the suite"
    );
    // The whole table, un-narrowed, would have paired three checks with
    // the suite's age.
    let all = parse_health(&published()).unwrap();
    assert_eq!(all.checks.len(), 3);
}

/// The Ops view READS the verdict: it never runs the suite. The suite is
/// the estate's own cron's business, and running it here paid 11 s of the
/// host's CPU for an answer the estate already had.
#[tokio::test]
async fn the_ops_view_never_runs_the_suite() {
    let (rd, wr, seen) = serve(|method, params| match method {
        "server/discover" => Reply::Result(discover(&[PROTOCOL_VERSION])),
        "tools/list" => Reply::Result(tools(&["health.list"])),
        "resources/read" => Reply::Result(json!({"contents": [{"text": published().to_string()}]})),
        "tools/call" => match params["name"].as_str().unwrap_or_default() {
            "health.list" => Reply::Result(json!({"structuredContent": suite()})),
            _ => Reply::Error(-32602, "unknown tool"),
        },
        _ => Reply::Error(-32601, "no"),
    });
    let mut client = Client::connect(rd, wr, std::time::Duration::from_secs(2))
        .await
        .unwrap();
    let s = gather(&mut client, NOW).await;
    let Part::Ready(h) = &s.health else {
        panic!("the health section did not read: {:?}", s.health);
    };
    assert_eq!(h.checks.len(), 2, "the suite's two checks");
    assert_eq!(h.oldest_age.as_deref(), Some("10 min ago"));
    let asked: Vec<String> = seen
        .lock()
        .unwrap()
        .iter()
        .filter_map(|r| r["params"]["name"].as_str().map(str::to_string))
        .collect();
    assert!(
        !asked.iter().any(|t| t == "health.run_all"),
        "the Ops view ran the suite: {asked:?}"
    );
    assert!(
        !asked.iter().any(|t| t == "health.run"),
        "the Ops view ran a check: {asked:?}"
    );
}

/// A check that failed names its own age, in the server's words.
#[test]
fn a_failing_check_names_its_own_age() {
    let mut h = verdict();
    h.checks[1].ok = false;
    h.checks[1].rc = 1;
    h.checks[1].age = Some("15 min ago".into());
    let mut s = snapshot(&h);
    s.cron = Part::Ready(Vec::new());
    s.containers = Part::Absent("no Docker on this host".into());
    s.alerts = Part::Ready(Vec::new());
    let f = plain(&render(
        "media-server",
        &OpsState::Ready(Box::new(s)),
        80,
        40,
        NOW,
        &pal(),
    ));
    assert!(
        f.iter()
            .any(|l| l.contains("check_vpn_leak  rc 1, 15 min ago")),
        "the failing check does not carry its age: {f:?}"
    );
    assert!(
        f.iter().any(|l| l.contains("1 of 2 failing · 10 min ago")),
        "the section does not carry the machine's age: {f:?}"
    );
}

/// The threshold is the estate's own cadence, plus a grace: a verdict read
/// between two of the estate's runs is at most the interval old.
#[test]
fn the_stale_threshold_is_the_estates_own_cadence() {
    assert_eq!(SUITE_INTERVAL_MINUTES, 15);
    assert_eq!(HEALTH_STALE_AFTER_S, 20 * 60);
    let mut h = verdict();
    h.oldest_age_s = Some(HEALTH_STALE_AFTER_S);
    assert!(!h.is_stale(), "exactly at the threshold is not yet stale");
    h.oldest_age_s = Some(HEALTH_STALE_AFTER_S + 1);
    assert!(h.is_stale(), "past the threshold is stale");
}

/// A check the suite does not run, and a check with no instance at all,
/// are both matched by the suite's own list.
#[test]
fn the_suite_matches_on_the_instance_when_both_name_one() {
    let suite = parse_suite(&suite()).unwrap();
    assert!(suite.takes(Some("media_server"), "native:check_disk_space"));
    assert!(
        !suite.takes(Some("solver"), "native:check_disk_space"),
        "another instance's check is not this one's"
    );
    assert!(
        !suite.takes(Some("media_server"), "native:check_image_drift_sweep"),
        "a daily check is not in the suite"
    );
}

/// The `Check` a client keeps carries the instance it came from, so a
/// machine with two stacks can tell its two `check_native_binaries` apart.
#[test]
fn a_check_keeps_the_instance_it_came_from() {
    let h = parse_health(&published()).unwrap();
    assert_eq!(h.checks[0].instance.as_deref(), Some("media_server"));
    assert_eq!(h.checks[0].age.as_deref(), Some("10 min ago"));
    let _ = Check {
        name: "x".into(),
        instance: None,
        ok: true,
        rc: 0,
        ts: 0,
        age: None,
    };
}
