//! The Ops frame, printed for a host: the one command a parity check can
//! read (servers ROADMAP O90).
//!
//! The monitor draws its Ops view in its TUI and keeps no copy of the
//! drawing, so the check that compares the two clients' panels had to
//! read the monitor's screen through tmux and say out loud that the
//! rendered half was a stand-in. This is the seam: one command that
//! prints the frame for a host, uncoloured, from the client's own cache -
//! the same lines the view draws, from the same answers.

use serde_json::json;

use super::*;
use crate::ops::cache::{Cache, Raw, RawSnapshot};

const NOW: i64 = 1_790_170_000;

/// A host, as the config carries it.
fn server() -> config::Server {
    config::Server {
        host: "192.168.0.33".into(),
        name: Some("beelink".into()),
        port: 22,
        user: "ztomer".into(),
        mcp: Some("/srv/media_server/bin/mcp_host --root /srv/media_server".into()),
        ..config::Server::default()
    }
}

/// The answer this client's cache holds for the host: the published
/// verdict, with the cadence beside the age.
fn answer() -> RawSnapshot {
    RawSnapshot {
        server: ("mcp_host".into(), "1.99.13".into()),
        at: NOW - 12,
        rule_every_s: 450,
        sampled_every_s: 450,
        catalogue: crate::mcp::catalogue::Catalogue::Listed { ttl_s: 3_600 },
        health: Raw::Value(json!({"checks": [
            {"instance": "media_server", "check": "native:check_disk_space", "ok": true, "rc": 0, "ms": 128, "ts": NOW - 600, "age_s": 600, "age": "10 min ago"},
        ], "oldest_age_s": 600, "oldest_age": "10 min ago",
        "cadence_s": 900, "cadence": "every 15 min"})),
        health_list: Raw::Value(json!({"checks": [
            {"instance": "media_server", "name": "native:check_disk_space", "schedule": "*/15 * * * *", "in_suite": true},
        ]})),
        vpn: Raw::Absent("this host lists no net.egress".into()),
        cron: Raw::Value(json!({"jobs": []})),
        containers: Raw::Absent("no Docker on this host".into()),
        zfs: Raw::Absent("this host lists no zfs.health".into()),
        disk: Raw::Absent("this host lists no disk.usage".into()),
        facts: Raw::Absent("this host lists no host.facts".into()),
        alerts: Raw::Value(json!({"events": []})),
        triage: Raw::Absent("this host lists no machine.triage".into()),
    }
}

/// A cache holding the answer above, and the config path beside it.
fn cached(dir: &std::path::Path) -> (PathBuf, Cache) {
    let config_path = dir.join("config.toml");
    let cache = Cache::beside(&config_path);
    let key = crate::ops::cache::host_key(&server());
    cache.write(&key, &answer()).expect("the answer is kept");
    (config_path, cache)
}

/// **THE FRAME IS PRINTED FOR A HOST.** The command the parity check
/// reads: the lines the view draws, for one host.
#[test]
fn the_frame_is_printed_for_a_host() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let (config_path, _cache) = cached(dir.path());
    let lines = ops_frame_lines(&[server()], &config_path, "beelink")
        .expect("the frame is printed for the host's name");
    let said = lines.join("\n");
    assert!(
        said.contains(&multitop_agent::fmt::fullwidth("beelink")),
        "the frame does not name the host: {said}"
    );
    assert!(
        said.contains("health"),
        "the frame does not draw the health section: {said}"
    );
    assert!(
        said.contains("10 min ago"),
        "the frame does not carry the verdict's age: {said}"
    );
}

/// **THE FRAME IS UNCOLOURED.** A frame a checker can read is a frame
/// with no escape codes in it.
#[test]
fn the_frame_is_uncoloured() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let (config_path, _cache) = cached(dir.path());
    let lines = ops_frame_lines(&[server()], &config_path, "beelink").expect("the frame");
    for line in &lines {
        assert!(
            !line.contains('\u{1b}'),
            "the frame carries an escape code: {line:?}"
        );
    }
}

/// **THE HOST IS NAMED BY ITS NAME OR BY ITS ADDRESS.** The parity check
/// knows the machine by the name the operator gave it; the operator knows
/// it by its address. Both find the same frame.
#[test]
fn the_host_is_found_by_its_name_or_its_address() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let (config_path, _cache) = cached(dir.path());
    let by_name = ops_frame_lines(&[server()], &config_path, "beelink").expect("by name");
    let by_address =
        ops_frame_lines(&[server()], &config_path, "192.168.0.33").expect("by address");
    assert_eq!(by_name, by_address, "the same host, the same frame");
}

/// **A HOST THAT IS NOT THERE IS REFUSED BY NAME.** A frame for nothing
/// is not a frame.
#[test]
fn a_host_that_is_not_configured_is_refused() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let (config_path, _cache) = cached(dir.path());
    let why = ops_frame_lines(&[server()], &config_path, "nope")
        .expect_err("a host that is not configured has no frame");
    assert!(why.contains("nope"), "the refusal names the host: {why}");
}

/// **A HOST WITH NO ANSWER SAYS SO.** The frame is drawn from the
/// client's own answers; a host it has never asked has none.
#[test]
fn a_host_with_no_answer_has_no_frame() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let config_path = dir.path().join("config.toml");
    let why = ops_frame_lines(&[server()], &config_path, "beelink")
        .expect_err("a host with no answer has no frame");
    assert!(why.contains("beelink"), "the refusal names the host: {why}");
}
