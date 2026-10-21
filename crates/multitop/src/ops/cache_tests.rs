//! The cache: what the last gather found, kept so the poll can run without
//! a terminal watching it.
//!
//! Three things are proved here, and each of them is a way the cache could
//! go wrong:
//!
//! - an answer kept is the answer read back, byte for byte in meaning: the
//!   raw answers are the server's own, and parsing them again gives the
//!   same snapshot the live gather gave;
//! - a file that cannot be read is NO answer, not a crash and not a wrong
//!   answer - the poll gathers again;
//! - an answer that is older than the one already kept does not replace it,
//!   because a write that lands late must not undo a newer answer.

use std::time::Duration;

use serde_json::json;

use super::cache::*;

/// The poll's interval, in seconds: the rule.
const EVERY: i64 = 450;

/// A cache in a directory of its own.
fn cache(dir: &tempfile::TempDir) -> Cache {
    Cache::beside(&dir.path().join("config.toml"))
}

/// An answer, as a gather would have left it.
fn raw(at: i64) -> RawSnapshot {
    RawSnapshot {
        server: ("mcp_host".into(), "1.99.11".into()),
        at,
        sampled_every_s: EVERY,
        health: Raw::Value(json!({"checks": [], "oldest_age": "10 min ago"})),
        health_list: Raw::Value(json!({"checks": []})),
        vpn: Raw::Absent("this host lists no net.egress".into()),
        cron: Raw::Value(json!({"jobs": []})),
        containers: Raw::Absent("no Docker on this host".into()),
        zfs: Raw::Absent("this host lists no zfs.health".into()),
        disk: Raw::Absent("this host lists no disk.usage".into()),
        facts: Raw::Absent("this host lists no host.facts".into()),
        alerts: Raw::Value(json!({"events": []})),
    }
}

/// An answer kept is the answer read back.
///
/// The comparison is the TEXT the cache holds: the raw answers carry a
/// `serde_json::Value`, which is not `Eq`, and an answer that compares
/// "nearly" is not an answer.
#[test]
fn an_answer_kept_is_the_answer_read_back() {
    let dir = tempfile::tempdir().unwrap();
    let cache = cache(&dir);
    cache.write("ztomer@192.168.0.33:22", &raw(1_000)).unwrap();
    let back = cache.read("ztomer@192.168.0.33:22").expect("the answer");
    assert_eq!(text(&back), text(&raw(1_000)));
}

/// An answer, as the cache holds it: the text.
fn text(answer: &RawSnapshot) -> String {
    serde_json::to_string(answer).expect("the answer is JSON")
}

/// A file that cannot be read is NO answer. It is not a wrong answer, and it
/// is not a crash: the answer is gone, and the poll gathers it again.
#[test]
fn a_file_that_cannot_be_read_is_no_answer() {
    let dir = tempfile::tempdir().unwrap();
    let cache = cache(&dir);
    let path = cache.path("ztomer@192.168.0.33:22").expect("a path");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "{not json at all").unwrap();
    assert!(cache.read("ztomer@192.168.0.33:22").is_none());
}

/// An answer that is OLDER than the one already kept does not replace it:
/// a write that lands late must not undo a newer answer.
#[test]
fn an_older_answer_does_not_replace_a_newer_one() {
    let dir = tempfile::tempdir().unwrap();
    let cache = cache(&dir);
    let key = "ztomer@192.168.0.33:22";
    cache.write(key, &raw(2_000)).unwrap();
    cache.write(key, &raw(1_000)).unwrap();
    assert_eq!(cache.read(key).expect("the answer").at, 2_000);
}

/// A host with no answer is DUE: the estate has never been asked.
#[test]
fn a_host_with_no_answer_is_due() {
    assert!(due(None, 1_000, Duration::from_secs(EVERY as u64)));
}

/// A host whose answer is older than the poll's interval is DUE: the estate
/// has published since, and the card would be showing an old machine.
#[test]
fn a_host_whose_answer_is_older_than_the_interval_is_due() {
    let old = raw(1_000);
    assert!(due(
        Some(&old),
        1_000 + EVERY,
        Duration::from_secs(EVERY as u64)
    ));
}

/// A host whose answer is inside the interval is NOT due: the estate has
/// published nothing new, and asking again would buy nothing.
#[test]
fn a_host_whose_answer_is_inside_the_interval_is_not_due() {
    let fresh = raw(1_000);
    let every = Duration::from_secs(EVERY as u64);
    let threshold = EVERY - DUE_SLACK_S;
    assert!(
        !due(Some(&fresh), 1_000 + threshold - 1, every),
        "an answer inside the slack is not due"
    );
    assert!(
        due(Some(&fresh), 1_000 + threshold, every),
        "an answer past the slack is due"
    );
}

/// The FRESH answer is the one the card shows; a stale one is not, whatever
/// it says.
#[test]
fn only_a_fresh_answer_is_shown_as_the_machines_answer() {
    let answer = raw(1_000);
    let every = Duration::from_secs(EVERY as u64);
    assert!(cache_read_fresh(&answer, 1_000 + EVERY - 1, every));
    assert!(!cache_read_fresh(&answer, 1_000 + EVERY, every));
}

/// The helper the cache uses, named so the test can ask it directly.
fn cache_read_fresh(answer: &RawSnapshot, now: i64, every: Duration) -> bool {
    is_fresh(answer, now, every)
}

/// **THE SCHEDULER'S TICK MUST NOT HALVE THE CLIENT'S RATE.** The pass runs
/// on the scheduler's clock, and a tick that lands a second early must not
/// skip the gather: the pass would then wait a whole extra cycle, and the
/// client would read the estate at half the rate the ruling asks - without
/// anything saying so. The slack is taken OFF the interval, so the client
/// reads MORE often than the rule, never less.
#[test]
fn a_gather_is_due_a_little_before_the_interval_is_up() {
    let answer = raw(1_000);
    let every = Duration::from_secs(EVERY as u64);
    let threshold = 1_000 + EVERY - DUE_SLACK_S;
    assert!(
        due(Some(&answer), threshold, every),
        "a tick that lands a second early skipped the gather"
    );
    assert!(
        !due(Some(&answer), threshold - 1, every),
        "the slack took the interval with it"
    );
}

/// **AN ANSWER SAYS WHEN THE CLIENT MISSED ITS SCHEDULE.** The rate an
/// answer carries is the poll's own - the rule's number - unless the gap
/// since the last answer is LONGER, in which case the gap is the truth, and
/// the truth is what the answer says. A client that slept through a cycle
/// must not carry a rate it did not keep: the age on the line cannot say it
/// (DECISIONS D15).
#[test]
fn an_answer_carries_the_rate_the_client_actually_kept() {
    assert_eq!(
        sampled_every_s(EVERY, None, 1_000),
        EVERY,
        "the first answer carries the rule"
    );
    assert_eq!(
        sampled_every_s(EVERY, Some(1_000), 1_420),
        EVERY,
        "a gather inside the rule carries the rule"
    );
    assert_eq!(
        sampled_every_s(EVERY, Some(1_000), 1_600),
        600,
        "a missed cycle carries the truth"
    );
}

/// A host the cache knows nothing about is due, and that is the whole of
/// the first pass: the estate has never been asked.
#[test]
fn a_host_the_cache_knows_nothing_about_is_due() {
    assert!(due(None, 1_000, Duration::from_secs(EVERY as u64)));
}

/// A host in the cache: the key a host is filed under is the same shape the
/// state file uses, so one host is one key everywhere.
#[test]
fn a_host_is_filed_under_its_own_key() {
    let server = crate::config::Server {
        host: "192.168.0.33".into(),
        port: 22,
        user: "ztomer".into(),
        name: None,
        upgrade_cmd: None,
        custom_command: None,
        mcp: Some("/bin/mcp_host".into()),
    };
    assert_eq!(host_key(&server), "ztomer@192.168.0.33:22");
    assert_eq!(
        file_name("ztomer@192.168.0.33:22"),
        "ztomer@192.168.0.33_22.json"
    );
}
