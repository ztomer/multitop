//! The agent's TICK: the rate the installed agent keeps, derived from the
//! cadence the estate publishes.
//!
//! servers ROADMAP E53: the installed agent's tick was a number this repo
//! carried, and launchd fixes an interval when the job is loaded - so the
//! tick could not follow the estate, and a cadence that moved left the
//! agent reading too slowly with nothing to do about it but reinstall.
//!
//! These tests pin the four things the tick has to be:
//!
//! - it FOLLOWS the cadence the estate published (and is not a number of
//!   this client's own);
//! - it is the FASTEST rate any host needs, because one agent reads them
//!   all;
//! - a cadence it could NOT read is STATED, and the rate it was keeping is
//!   kept - never silently replaced by a hard-coded number called the
//!   published rate;
//! - it says WHERE the cadence came from, so the log can be checked.

use serde_json::json;

use super::cache::{Cache, Raw, RawSnapshot};
use super::tick::*;
use crate::config::Server;

/// The epoch second an answer was taken at.
const AT: i64 = 1_790_170_000;

/// The rule the client keeps for a fifteen-minute publication.
const EVERY: i64 = 450;

/// An answer, as a gather would have left it.
fn raw(at: i64) -> RawSnapshot {
    RawSnapshot {
        server: ("mcp_host".into(), "1.99.21".into()),
        at,
        rule_every_s: EVERY,
        sampled_every_s: EVERY,
        catalogue: crate::mcp::catalogue::Catalogue::Listed { ttl_s: 3_600 },
        health: Raw::Value(json!({"checks": [], "oldest_age": "10 min ago"})),
        health_list: Raw::Value(json!({"checks": []})),
        vpn: Raw::Absent("this host lists no net.egress".into()),
        cron: Raw::Value(json!({"jobs": []})),
        containers: Raw::Absent("no Docker on this machine".into()),
        zfs: Raw::Absent("this host lists no zfs.health".into()),
        disk: Raw::Absent("this host lists no disk.usage".into()),
        facts: Raw::Absent("this host lists no host.facts".into()),
        alerts: Raw::Value(json!({"events": []})),
        triage: Raw::Absent("this host lists no machine.triage".into()),
    }
}

/// An answer whose verdict carried a cadence.
fn published(at: i64, cadence_s: i64) -> RawSnapshot {
    let mut answer = raw(at);
    answer.health = Raw::Value(json!({
        "checks": [],
        "oldest_age": "10 min ago",
        "cadence_s": cadence_s,
        "cadence": "every 15 min",
    }));
    answer.rule_every_s = cadence_s / 2;
    answer
}

/// A host in the config.
fn server(host: &str) -> Server {
    Server {
        host: host.into(),
        port: 22,
        user: "ztomer".into(),
        name: None,
        upgrade_cmd: None,
        custom_command: None,
        mcp: Some("/bin/mcp_host --root /srv".into()),
    }
}

/// A cache in a directory of its own.
fn cache(dir: &tempfile::TempDir) -> Cache {
    Cache::beside(&dir.path().join("config.toml"))
}

/// **THE TICK FOLLOWS THE CADENCE THE ESTATE PUBLISHED.** A cadence of 900
/// is a rate of 450 (the rule, DECISIONS D15); a cadence of 300 is a rate
/// of 150. The tick is that rate, less the margin - and it is NOT a number
/// this repo carries: the two cadences give two different ticks.
#[test]
fn the_tick_follows_the_cadence_the_estate_published() {
    let quarter = rate_of("192.168.0.33", Some(&published(AT, 900)));
    assert_eq!(
        quarter.rate_s, 450,
        "the rule is half the published cadence"
    );
    assert_eq!(quarter.source, Source::Published { cadence_s: 900 });

    let fifth = rate_of("192.168.0.33", Some(&published(AT, 300)));
    assert_eq!(fifth.rate_s, 150, "a cadence that moves moves the rate");
    assert_eq!(fifth.source, Source::Published { cadence_s: 300 });

    let slow = tick_of(&[quarter]);
    let fast = tick_of(&[fifth]);
    assert_eq!(
        slow.secs,
        450 - TICK_MARGIN_S,
        "the tick is the rule, less the margin"
    );
    assert_eq!(
        fast.secs,
        150 - TICK_MARGIN_S,
        "the tick follows the cadence down"
    );
    assert!(
        slow.secs > fast.secs,
        "a faster publication did not make the tick faster"
    );
}

/// **THE TICK IS THE FASTEST RATE ANY HOST NEEDS.** One agent reads every
/// host, so one tick serves them all: the shortest cadence any host
/// published, halved.
#[test]
fn the_tick_is_the_fastest_rate_any_host_needs() {
    let rates = vec![
        rate_of("192.168.0.33", Some(&published(AT, 900))),
        rate_of("192.168.0.90", Some(&published(AT, 300))),
        rate_of("192.168.0.158", Some(&published(AT, 900))),
    ];
    let tick = tick_of(&rates);
    assert_eq!(
        tick.secs,
        150 - TICK_MARGIN_S,
        "the fastest host set the tick"
    );
    assert_eq!(
        tick.host, "192.168.0.90",
        "the tick names the host it follows"
    );
}

/// **A CADENCE THAT COULD NOT BE READ IS STATED, AND THE RATE THE CLIENT
/// WAS KEEPING IS KEPT.** A host that published no cadence is not a host
/// with no rate: the client keeps the rate it had, and says so.
#[test]
fn a_host_that_published_no_cadence_keeps_the_rate_it_had() {
    let rate = rate_of("127.0.0.1", Some(&raw(AT)));
    assert_eq!(rate.rate_s, EVERY, "the rate the client was keeping");
    assert_eq!(rate.source, Source::Kept { rate_s: EVERY });

    let tick = tick_of(&[rate]);
    assert_eq!(
        tick.secs,
        EVERY - TICK_MARGIN_S,
        "the kept rate is the tick"
    );
    assert_eq!(tick.unread.len(), 1, "the host is named as unread");
    assert_eq!(tick.unread[0].host, "127.0.0.1");
}

/// **A HOST THIS CLIENT HAS NEVER READ KEEPS THE CLIENT'S OWN FALLBACK** -
/// and says so, rather than passing the fallback off as a published rate.
#[test]
fn a_host_this_client_has_never_read_keeps_the_fallback() {
    let rate = rate_of("192.168.0.99", None);
    assert_eq!(rate.rate_s, super::rate::FALLBACK_INTERVAL_S);
    assert_eq!(
        rate.source,
        Source::Fallback {
            rate_s: super::rate::FALLBACK_INTERVAL_S
        }
    );
}

/// **THE TICK SAYS WHERE THE CADENCE CAME FROM.** A number in a log that
/// names nothing cannot be checked.
#[test]
fn the_tick_says_where_the_cadence_came_from() {
    let tick = tick_of(&[rate_of("192.168.0.33", Some(&published(AT, 900)))]);
    let line = tick.line(AT + 12);
    assert!(
        line.contains("every 15 min"),
        "the tick does not say what the estate publishes: {line}"
    );
    assert!(
        line.contains("health://latest"),
        "the tick does not say where the cadence was read: {line}"
    );
    assert!(
        line.contains("192.168.0.33"),
        "the tick does not name the host it read the cadence from: {line}"
    );
    assert!(
        line.contains("12 s ago"),
        "the tick does not say how old the cadence is: {line}"
    );
}

/// **A CADENCE THAT COULD NOT BE READ IS SAID IN WORDS, NOT AS A NUMBER.**
/// The agent says it is keeping a rate, and where that rate came from.
#[test]
fn a_cadence_that_could_not_be_read_is_said_in_words() {
    let tick = tick_of(&[rate_of("127.0.0.1", Some(&raw(AT)))]);
    let line = tick.line(AT);
    assert!(
        line.contains("the cadence could not be read"),
        "the tick does not say the cadence could not be read: {line}"
    );
    assert!(
        line.contains("keeps its rate"),
        "the tick does not say what it keeps instead: {line}"
    );
    let note = tick.unread[0].note();
    assert!(
        note.contains("the cadence could not be read"),
        "the host's note does not say the cadence could not be read: {note}"
    );
}

/// **THE TICK IS READ FROM WHAT THE CLIENT HAS READ.** The cache holds the
/// answers the last pass kept, and the cadence travels with them.
#[test]
fn the_tick_is_read_from_the_answers_the_client_kept() {
    let dir = tempfile::tempdir().unwrap();
    let cache = cache(&dir);
    let servers = vec![server("192.168.0.33"), server("127.0.0.1")];
    cache
        .write("ztomer@192.168.0.33:22", &published(AT, 300))
        .unwrap();
    cache.write("ztomer@127.0.0.1:22", &raw(AT)).unwrap();

    let tick = tick_for(&servers, &cache);
    assert_eq!(
        tick.secs,
        150 - TICK_MARGIN_S,
        "the tick did not follow the cadence the kept answers carry"
    );
    assert_eq!(tick.host, "192.168.0.33");
    assert_eq!(tick.unread.len(), 1, "the host that published no cadence");
    assert_eq!(tick.unread[0].host, "127.0.0.1");
}

/// **A HOST THAT PUBLISHED A CADENCE SAYS SO IN WORDS.** The note is what
/// the log carries for a host, and a host that published one is not a host
/// whose cadence could not be read.
#[test]
fn a_host_that_published_a_cadence_says_so() {
    let rate = rate_of("192.168.0.33", Some(&published(AT, 900)));
    assert!(
        rate.note().contains("every 15 min"),
        "the note does not say what the host published: {}",
        rate.note()
    );
}

/// **AN ESTATE WITH NO HOST AT ALL IS SAID, NOT GUESSED.** The agent keeps
/// the client's own fallback rate, and the log says that no host has ever
/// been read - it does not print a rate as if a host had published it.
#[test]
fn an_estate_with_no_host_is_said_not_guessed() {
    let tick = tick_of(&[]);
    assert_eq!(
        tick.secs,
        super::rate::FALLBACK_INTERVAL_S - TICK_MARGIN_S,
        "the agent kept the client's fallback rate"
    );
    assert_eq!(tick.host.len(), 0, "a host was named that does not exist");
    let line = tick.line(AT);
    assert!(
        line.contains("no host has ever been read"),
        "the tick does not say that no host was read: {line}"
    );
}

/// **THE AGE OF THE READING IS IN WORDS.** A cadence that is an hour old
/// and one that is a second old are not the same fact, and the log says
/// which it is.
#[test]
fn the_age_of_the_reading_is_in_words() {
    let tick = tick_of(&[rate_of("192.168.0.33", Some(&published(AT, 900)))]);
    assert!(tick.line(AT + 1).contains("1 s ago"), "a second old");
    assert!(tick.line(AT + 90).contains("1 min ago"), "a minute old");
    assert!(tick.line(AT + 7_200).contains("2 h ago"), "two hours old");
    assert!(
        tick.line(AT + 3 * 86_400).contains("3 days ago"),
        "three days old"
    );
}

/// **THE TICK NAMES A HOST THAT PUBLISHED ITS CADENCE.** A host that merely
/// kept a rate has published nothing, so a tie must not be settled by it:
/// the tick's own line says where the cadence came from, and that has to be
/// a reading, not a kept rate.
#[test]
fn the_tick_names_a_host_that_published_its_cadence() {
    let rates = vec![
        rate_of("127.0.0.1", Some(&raw(AT))),
        rate_of("192.168.0.33", Some(&published(AT, 900))),
    ];
    let tick = tick_of(&rates);
    assert_eq!(
        tick.secs,
        450 - TICK_MARGIN_S,
        "both hosts need the same rate"
    );
    assert_eq!(
        tick.host, "192.168.0.33",
        "the tick names a host that kept a rate instead of one that published one"
    );
    assert_eq!(tick.source, Source::Published { cadence_s: 900 });

    // Two hosts that published the same cadence: the first one keeps the
    // tick, so the log names the same host every cycle.
    let rates = vec![
        rate_of("192.168.0.33", Some(&published(AT, 900))),
        rate_of("192.168.0.90", Some(&published(AT, 900))),
    ];
    assert_eq!(tick_of(&rates).host, "192.168.0.33");
}
