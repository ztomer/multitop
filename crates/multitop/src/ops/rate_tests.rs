//! The client's sampling rate is DERIVED from the cadence the server
//! publishes, and never carried as a number of this client's own.
//!
//! servers ROADMAP E53, and DECISIONS D15 (owner, 2026-10-20): a client
//! that samples at the rate the estate publishes can ALIAS - its read
//! lands just before a publication and then waits a whole cycle, and its
//! verdict is two publications behind the truth. So a client samples at
//! TWICE the publication rate.
//!
//! The publication rate used to be a number COPIED into this client
//! (450 s - half of a fifteen-minute publication), which is why a server
//! that published every five minutes would not have been noticed here at
//! all: neither this client's rate nor its staleness threshold moved, so
//! the aliasing the ruling forbids would have come back silently. The
//! cadence now travels WITH the verdict (`health://latest`, beside the
//! age), and everything below is derived from what was published.

use serde_json::json;

use super::answers::parse_health;
use super::gather::gather;
use super::model_health::Cadence;
use super::rate::Previous;
use super::*;
use crate::mcp::client::{Client, PROTOCOL_VERSION};
use crate::mcp::fake::{discover, serve, tools, Reply};

const NOW: i64 = 1_790_170_000;

/// The cadence the estate published: every fifteen minutes.
fn every_15() -> Cadence {
    Cadence {
        secs: Some(900),
        words: Some("every 15 min".into()),
    }
}

/// A cadence the server could not read: it says so, in its own words.
fn unknown() -> Cadence {
    Cadence {
        secs: None,
        words: Some("unknown".into()),
    }
}

/// The published verdict, as the estate's server publishes it, with the
/// cadence it published beside the age.
fn published(cadence_s: Option<i64>, cadence: &str) -> serde_json::Value {
    json!({"checks": [
        {"instance": "media_server", "check": "native:check_disk_space", "ok": true, "rc": 0, "ms": 128, "ts": NOW - 600, "age_s": 600, "age": "10 min ago"},
    ], "oldest_age_s": 600, "oldest_age": "10 min ago",
    "cadence_s": cadence_s, "cadence": cadence})
}

/// The machine's suite: the one periodic check is in it.
fn suite() -> serde_json::Value {
    json!({"checks": [
        {"instance": "media_server", "name": "native:check_disk_space", "schedule": "*/15 * * * *", "in_suite": true},
    ]})
}

/// A client over an in-memory server that publishes `published`.
async fn client_answering(
    published: serde_json::Value,
) -> Client<
    tokio::io::ReadHalf<tokio::io::DuplexStream>,
    tokio::io::WriteHalf<tokio::io::DuplexStream>,
> {
    let (rd, wr, _seen) = serve(move |method, params| match method {
        "server/discover" => Reply::Result(discover(&[PROTOCOL_VERSION])),
        "tools/list" => Reply::Result(tools(&["health.list"])),
        "resources/read" => Reply::Result(json!({"contents": [{"text": published.to_string()}]})),
        "tools/call" => match params["name"].as_str().unwrap_or_default() {
            "health.list" => Reply::Result(json!({"structuredContent": suite()})),
            _ => Reply::Error(-32602, "unknown tool"),
        },
        _ => Reply::Error(-32601, "no"),
    });
    Client::connect(rd, wr, std::time::Duration::from_secs(2))
        .await
        .unwrap()
}

/// A snapshot whose only interesting section is the health one.
fn snapshot_with(h: Health) -> Snapshot {
    Snapshot {
        server: ("mcp_host".into(), "1.99.13".into()),
        at: NOW - 12,
        sampled_every_s: 450,
        health: Part::Ready(h),
        vpn: Part::Absent("this host lists no net.egress".into()),
        cron: Part::Ready(Vec::new()),
        containers: Part::Absent("no Docker on this host".into()),
        zfs: Part::Absent("this host lists no zfs.health".into()),
        disk: Part::Absent("this host lists no disk.usage".into()),
        facts: Part::Absent("this host lists no host.facts".into()),
        alerts: Part::Ready(Vec::new()),
    }
}

/// **THE CADENCE IS PUBLISHED WITH THE VERDICT.** The server's own
/// number, in seconds, and its own words beside it.
#[test]
fn the_cadence_travels_with_the_verdict() {
    let h = parse_health(&published(Some(900), "every 15 min")).unwrap();
    assert_eq!(h.cadence.secs, Some(900), "the cadence in seconds");
    assert_eq!(h.cadence.say(), "every 15 min", "the server's own words");
    assert!(h.cadence.is_known(), "the cadence is known");
}

/// A server that could not read its own cadence says so, in seconds (null)
/// and in words ("unknown") - and the client cannot pretend to know it.
#[test]
fn a_cadence_the_server_could_not_read_is_unknown() {
    let h = parse_health(&published(None, "unknown")).unwrap();
    assert_eq!(h.cadence.secs, None, "no seconds");
    assert_eq!(h.cadence.say(), "unknown", "the server's own word");
    assert!(!h.cadence.is_known(), "the cadence is not known");
}

/// **THE RATE IS HALF THE PUBLISHED CADENCE.** The rule (D15), read from
/// the estate rather than copied into this client.
#[test]
fn the_rate_is_half_the_cadence_the_server_published() {
    assert_eq!(rate_for(900), 450, "every 15 min");
    assert_eq!(rate_for(300), 150, "every 5 min");
    assert_eq!(rate_for(60), 30, "every minute");
    assert_eq!(
        interval_for(None, None).as_secs(),
        u64::try_from(FALLBACK_INTERVAL_S).unwrap(),
        "no cadence, no rate to derive: the fallback"
    );
}

/// **A SERVER THAT PUBLISHES FASTER IS A DIFFERENT RATE.** The defect
/// E53 is about: a client that carries the interval as a number of its
/// own keeps the old rate when the estate's cron moves.
#[test]
fn a_faster_publication_is_a_faster_rate() {
    assert_ne!(
        rate_for(300),
        rate_for(900),
        "a server that publishes every five minutes is not a server that publishes every fifteen"
    );
    assert_eq!(
        interval_for(Some(300), None).as_secs(),
        150,
        "and the interval this client samples at follows it"
    );
}

/// **A CADENCE THAT COULD NOT BE READ KEEPS THE RATE THE CLIENT HAD.**
/// Not a warning: a gap in the reading is not a fault.
#[test]
fn a_cadence_that_could_not_be_read_keeps_the_rate_the_client_had() {
    assert_eq!(
        interval_for(None, Some(450)).as_secs(),
        450,
        "the rate the client had"
    );
    assert_eq!(
        interval_for(None, None).as_secs(),
        u64::try_from(FALLBACK_INTERVAL_S).unwrap(),
        "and a client that never had a rate falls back"
    );
}

/// **THE WARNING IS DRIVEN BY THE PUBLISHED CADENCE.** The rule is
/// relative to what the server publishes, not to a number this client
/// carries.
#[test]
fn the_warning_is_driven_by_the_published_cadence() {
    // Half of every 15 min: the client is inside the rule, and says
    // nothing.
    assert!(
        sampling_warning(450, &every_15()).is_none(),
        "a client at the rule warns anyway"
    );
    // The same client, against a server that publishes every 5 min: the
    // client cannot keep up, and says so.
    let fast = Cadence {
        secs: Some(300),
        words: Some("every 5 min".into()),
    };
    let warning = sampling_warning(450, &fast).expect("450 s is slower than 150 s");
    assert!(
        warning.contains("every 5 min"),
        "the warning names the cadence the server published: {warning}"
    );
    assert!(
        warning.contains("two publications behind"),
        "the warning does not say what it means: {warning}"
    );
}

/// **A CADENCE THE CLIENT COULD NOT READ IS NOT A WARNING.** The client
/// cannot tell whether it is keeping up, and a gap in the reading is not
/// a fault. What it says instead is that the rate could not be read.
#[test]
fn an_unknown_cadence_is_not_a_warning() {
    assert!(
        sampling_warning(900, &unknown()).is_none(),
        "an unknown cadence warns anyway"
    );
    let note = cadence_unknown_note(450);
    assert!(
        note.contains("could not be read"),
        "the client does not say the rate could not be read: {note}"
    );
}

/// **THE ANSWER CARRIES THE RATE THE PUBLISHED CADENCE IMPLIES.** The
/// whole point of E53: the client does not carry the interval, it reads
/// it.
#[tokio::test]
async fn the_answer_carries_the_rate_the_published_cadence_implies() {
    let mut client = client_answering(published(Some(300), "every 5 min")).await;
    let s = gather(&mut client, NOW, Previous::default()).await;
    assert_eq!(
        s.sampled_every_s, 150,
        "the rate is half of the published cadence"
    );
    let Part::Ready(h) = &s.health else {
        panic!("the health section did not read: {:?}", s.health);
    };
    assert_eq!(h.cadence.secs, Some(300), "the cadence is on the verdict");
}

/// **A CADENCE THAT COULD NOT BE READ KEEPS THE RATE THE CLIENT HAD.**
#[tokio::test]
async fn an_answer_whose_cadence_is_unknown_keeps_the_rate_it_had() {
    let mut client = client_answering(published(None, "unknown")).await;
    let prev = Previous {
        at: Some(NOW - 100),
        rate_s: Some(450),
    };
    let s = gather(&mut client, NOW, prev).await;
    assert_eq!(s.sampled_every_s, 450, "the rate the client had");
}

/// **A CLIENT THAT MISSED ITS SCHEDULE CARRIES THE GAP.** The rule's
/// number is the rate the client should keep; the gap is the rate it
/// kept.
#[tokio::test]
async fn an_answer_carries_the_gap_when_the_client_missed_its_schedule() {
    let mut client = client_answering(published(Some(900), "every 15 min")).await;
    let prev = Previous {
        at: Some(NOW - 900),
        rate_s: Some(450),
    };
    let s = gather(&mut client, NOW, prev).await;
    assert_eq!(s.sampled_every_s, 900, "the gap is the truth");
}

/// **THE STALE THRESHOLD FOLLOWS THE PUBLISHED CADENCE.** The verdict is
/// stale when it is older than a publication the estate would have made.
#[test]
fn the_stale_threshold_follows_the_published_cadence() {
    let mut h = parse_health(&published(Some(300), "every 5 min")).unwrap();
    h.oldest_age_s = Some(600);
    assert!(
        !h.is_stale(),
        "a verdict as old as the cadence plus its grace is not stale yet"
    );
    h.oldest_age_s = Some(601);
    assert!(h.is_stale(), "past the cadence plus its grace is stale");
    // The cadence the estate published a moment ago: the same verdict is
    // still fresh, because the estate has not published since.
    let mut h = parse_health(&published(Some(900), "every 15 min")).unwrap();
    h.oldest_age_s = Some(600);
    assert!(!h.is_stale(), "a fresh verdict is not stale");
}

/// **THE ANSWER SAYS WHEN THE CLIENT CANNOT KEEP UP.** The warning is
/// where the verdict is shown, because the age on the line cannot say it.
#[test]
fn the_frame_warns_when_the_client_cannot_keep_up() {
    let mut h = parse_health(&published(Some(300), "every 5 min")).unwrap();
    h.oldest_age_s = Some(600);
    let mut s = snapshot_with(h);
    s.sampled_every_s = 450;
    let frame = super::tests::plain(&super::render::render(
        "media-server",
        &OpsState::Ready(Box::new(s)),
        100,
        40,
        NOW,
        &super::tests::pal(),
    ));
    let said = frame.join("\n");
    assert!(
        said.contains("two publications behind"),
        "a client that cannot keep up does not say so: {said}"
    );
}
