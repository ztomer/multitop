//! The GATHER: asking a host every question the Ops view asks, and keeping
//! the answers.
//!
//! Two shapes of the same work, and the seam between them is the wire: the
//! raw gather is what the estate SAID, and the parse is what this client
//! draws of it. The cache keeps the first (servers ROADMAP E36: the poll
//! runs without a terminal, so what it found has to outlive the process),
//! and the view draws the second - so an answer read back from the cache is
//! parsed by exactly the code that parses a live answer.

use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncRead, AsyncWrite};

use super::cache;
use super::model::Health;
use super::{
    parse_alerts, parse_containers, parse_cron, parse_disk, parse_egress, parse_facts, parse_zfs,
};
use super::{
    parse_health, parse_suite, Part, Snapshot, ALERT_WINDOW_S, DISK_TOOL, EGRESS_TIMEOUT,
    EGRESS_TOOL, FACTS_TOOL, HEALTH_LIST, HEALTH_URI, ZFS_TOOL,
};
use crate::mcp::client::Client;

/// Ask a connected server for every section at `now` (epoch seconds).
///
/// `prev` is what this client knew before the gather: when the last
/// answer was taken, and at what rate. The rate this answer carries is
/// DERIVED from the cadence the server published with the verdict
/// (servers ROADMAP E53) - a client that samples slower than half the
/// publication rate says so where the verdict is shown (DECISIONS D15).
pub async fn gather<R, W>(c: &mut Client<R, W>, now: i64, prev: super::rate::Previous) -> Snapshot
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    parse_snapshot(&gather_raw(c, now, prev).await)
}

/// Ask a connected server for every section, and keep the ANSWERS.
///
/// This is the gather the cache keeps: the server's own words, not this
/// client's drawing of them. [`parse_snapshot`] turns them into the shape
/// the view draws, and it is the same parsing either way - so an answer read
/// from the cache is the answer the estate gave, not a copy of it.
pub async fn gather_raw<R, W>(
    c: &mut Client<R, W>,
    now: i64,
    prev: super::rate::Previous,
) -> cache::RawSnapshot
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let (health, health_list) = if c.has_tool(HEALTH_LIST) {
        health_raw(c).await
    } else {
        let why = c.absent_reason(HEALTH_LIST, "this host lists no health checks");
        (cache::Raw::Absent(why.clone()), cache::Raw::Absent(why))
    };
    let vpn = if c.has_tool(EGRESS_TOOL) {
        ask(c, EGRESS_TOOL, json!({}), EGRESS_TIMEOUT).await
    } else {
        cache::Raw::Absent(c.absent_reason(EGRESS_TOOL, "this host lists no net.egress"))
    };
    let cron = if c.has_tool("cron.status") {
        ask(c, "cron.status", json!({}), c.timeout()).await
    } else {
        cache::Raw::Absent(c.absent_reason("cron.status", "this host lists no cron.status"))
    };
    let containers = if c.has_tool("docker.containers") {
        ask(c, "docker.containers", json!({}), c.timeout()).await
    } else {
        cache::Raw::Absent(c.absent_reason("docker.containers", "no Docker on this host"))
    };
    let zfs = if c.has_tool(ZFS_TOOL) {
        ask(c, ZFS_TOOL, json!({}), c.timeout()).await
    } else {
        cache::Raw::Absent(c.absent_reason(ZFS_TOOL, "this host lists no zfs.health"))
    };
    let disk = if c.has_tool(DISK_TOOL) {
        ask(c, DISK_TOOL, json!({}), c.timeout()).await
    } else {
        cache::Raw::Absent(c.absent_reason(DISK_TOOL, "this host lists no disk.usage"))
    };
    let facts = if c.has_tool(FACTS_TOOL) {
        ask(c, FACTS_TOOL, json!({}), c.timeout()).await
    } else {
        cache::Raw::Absent(c.absent_reason(FACTS_TOOL, "this host lists no host.facts"))
    };
    let alerts = if c.has_tool("alerts.since") {
        ask(
            c,
            "alerts.since",
            json!({ "marker": now - ALERT_WINDOW_S }),
            c.timeout(),
        )
        .await
    } else {
        cache::Raw::Absent(c.absent_reason("alerts.since", "this host lists no alerts.since"))
    };
    // The rate the answer carries is DERIVED, from the cadence the server
    // published with the verdict (servers ROADMAP E53). A server that
    // could not read its own cadence leaves the client with the rate it
    // was keeping - and the gap since the last answer, when the client
    // missed its schedule, is the truth about the rate it kept.
    let rule_every_s = cadence_of(&health)
        .map(super::rate::rate_for)
        .or(prev.rate_s)
        .unwrap_or(super::rate::FALLBACK_INTERVAL_S);
    let sampled_every_s = super::rate::carried_rate(rule_every_s, prev.at, now);
    cache::RawSnapshot {
        server: c.server.clone(),
        at: now,
        rule_every_s,
        sampled_every_s,
        catalogue: c.catalogue.clone(),
        health,
        health_list,
        vpn,
        cron,
        containers,
        zfs,
        disk,
        facts,
        alerts,
    }
}

/// The cadence the server published with the verdict, in seconds.
///
/// The server's own number, read from the answer it gave. `None` when the
/// server could not read its own cadence - and when the verdict itself
/// could not be read, because then there is nothing to read it from.
fn cadence_of(health: &cache::Raw) -> Option<i64> {
    health.value()?.get("cadence_s").and_then(Value::as_i64)
}

/// The shape the view draws, from the server's own words.
///
/// The health section is TWO answers - the published verdict and the suite
/// it belongs to - so it is the one section read from two fields at once.
/// A verdict that cannot be narrowed to its suite is not shown at all: the
/// published table carries the daily checks too, whose verdicts are a day
/// old by design, and a count that included them would be paired with an
/// age that does not describe them.
#[must_use]
pub fn parse_snapshot(raw: &cache::RawSnapshot) -> Snapshot {
    Snapshot {
        server: raw.server.clone(),
        at: raw.at,
        sampled_every_s: raw.sampled_every_s,
        catalogue: raw.catalogue.clone(),
        health: parse_health_raw(&raw.health, &raw.health_list),
        vpn: parse_part(&raw.vpn, parse_egress),
        cron: parse_part(&raw.cron, parse_cron),
        containers: parse_part(&raw.containers, parse_containers),
        zfs: parse_part(&raw.zfs, parse_zfs),
        disk: parse_part(&raw.disk, parse_disk),
        facts: parse_part(&raw.facts, parse_facts),
        alerts: parse_part(&raw.alerts, parse_alerts),
    }
}

/// One answer, as the shape the view draws.
fn parse_part<T>(raw: &cache::Raw, parse: fn(&Value) -> Result<T, String>) -> Part<T> {
    match raw {
        cache::Raw::Value(v) => parse(v).map_or_else(Part::Failed, Part::Ready),
        cache::Raw::Failed(why) => Part::Failed(why.clone()),
        cache::Raw::Absent(why) => Part::Absent(why.clone()),
        cache::Raw::Unknown(why) => Part::Unknown(why.clone()),
    }
}

/// The health section's two answers: the PUBLISHED verdict and the suite's
/// own list.
///
/// The verdict is READ, never run (servers ROADMAP E12): the estate's own
/// cron runs the suite every 15 minutes and publishes what it found, and
/// running it here would pay the suite's whole cost for an answer the
/// estate already has. The suite's list is asked too, so the verdict is the
/// suite's - the published table also carries the daily checks, whose
/// verdicts are a day old by design and would pair a count with an age that
/// does not describe it.
///
/// A section that cannot be read says why: an absent verdict is never an
/// all-clear.
async fn health_raw<R, W>(c: &mut Client<R, W>) -> (cache::Raw, cache::Raw)
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    if let Some(why) = c.broken() {
        let why = format!("not read: the session ended: {why}");
        return (cache::Raw::Unknown(why.clone()), cache::Raw::Unknown(why));
    }
    let published = match c.read(HEALTH_URI).await {
        Ok(v) => v,
        Err(e) => return (raw(Err(e)), cache::Raw::Value(Value::Null)),
    };
    let listed = match c.call(HEALTH_LIST, json!({})).await {
        Ok(v) => v,
        Err(e) => return (cache::Raw::Value(published), raw(Err(e))),
    };
    (cache::Raw::Value(published), cache::Raw::Value(listed))
}

/// The health section, from the published verdict and the suite's list.
fn parse_health_raw(published: &cache::Raw, listed: &cache::Raw) -> Part<Health> {
    let (Some(published), Some(listed)) = (published.value(), listed.value()) else {
        return match (published, listed) {
            (cache::Raw::Failed(why), _) | (_, cache::Raw::Failed(why)) => {
                Part::Failed(why.clone())
            }
            (cache::Raw::Unknown(why), _) | (_, cache::Raw::Unknown(why)) => {
                Part::Unknown(why.clone())
            }
            (cache::Raw::Absent(why), _) | (_, cache::Raw::Absent(why)) => {
                Part::Absent(why.clone())
            }
            _ => Part::Failed("the health section has no answer".into()),
        };
    };
    let mut health = match parse_health(published) {
        Ok(h) => h,
        Err(why) => return Part::Failed(why),
    };
    let suite = match parse_suite(listed) {
        Ok(s) => s,
        Err(why) => return Part::Failed(why),
    };
    health.narrow_to(&suite);
    Part::Ready(health)
}

/// One section's answer, asked of a connected server.
///
/// **A MISSED DEADLINE IS A GAP IN ONE SECTION.** The answer did not
/// arrive, so the fact is unknown - and it is unknown because this
/// client did not hear, NOT because the estate is broken (servers
/// ROADMAP E74). The session is still good after a missed deadline, so
/// the estate's other readings are still read.
async fn ask<R, W>(c: &mut Client<R, W>, tool: &str, args: Value, timeout: Duration) -> cache::Raw
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    if let Some(why) = c.broken() {
        // The session is gone: this section was never asked, and the
        // reason is the session's end - not this section's own.
        return cache::Raw::Unknown(format!("not read: the session ended: {why}"));
    }
    raw(c.call_within(tool, args, timeout).await)
}

/// An answer, as the server gave it - or the reason there is none.
fn raw(answer: Result<Value, crate::mcp::client::Error>) -> cache::Raw {
    match answer {
        Ok(v) => cache::Raw::Value(v),
        // The answer did not arrive: the fact is UNKNOWN, and the reason
        // says so. The estate's own report of a failure is a FINDING, and
        // stays one.
        Err(e) if e.is_unread() => cache::Raw::Unknown(e.to_string()),
        Err(e) => cache::Raw::Failed(e.to_string()),
    }
}
