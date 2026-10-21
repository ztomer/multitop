//! The Ops view's ZFS section: the tool that owns the pools' state, read.
//!
//! Three hosts, three facts: a host that serves `zfs.health` (its pools are
//! read and a pool that is not whole is VISIBLE), a host that does not (the
//! tool is named, and the server's own reason says why), and a host whose
//! answer cannot be read (said, never guessed).

use std::time::Duration;

use serde_json::{json, Value};

use super::render::render;
use super::tests::{pal, plain};
use super::*;
use crate::mcp::client::{Client, PROTOCOL_VERSION};
use crate::mcp::fake::{discover_absent, serve, tools, Reply};

const NOW: i64 = 1_790_170_000;

/// The reason `.158`'s `mcp_host` publishes for a host with no ZFS.
const NO_ZFS: &str = "no ZFS on this host: no /usr/sbin/zpool";

/// The pools of a host whose pool0 lost a member (the 2026-10-19 incident on
/// `.90`), as `zfs.health` published them.
fn broken_pools() -> Value {
    json!({"pools": [
        {"name": "pool0", "state": "SUSPENDED", "action": "attach the missing device",
         "msg": "One or more devices could not be opened", "scan": "scrub repaired 0B in 00:00:01",
         "members": [
             {"name": "mirror-0", "state": "DEGRADED", "read": 0, "write": 0, "cksum": 0},
             {"name": "sdc", "state": "REMOVED", "read": 0, "write": 0, "cksum": 0},
         ]},
    ]})
}

/// The pools of a healthy host, as `.33` published them.
fn healthy_pools() -> Value {
    json!({"pools": [
        {"name": "pool0", "state": "ONLINE", "action": "", "msg": "", "scan": "scrub repaired 0B",
         "members": [
             {"name": "mirror-0", "state": "ONLINE", "read": 0, "write": 0, "cksum": 0},
             {"name": "usb-Seagate-0", "state": "ONLINE", "read": 0, "write": 0, "cksum": 0},
         ]},
    ]})
}

/// A host that lists `listing` and answers `zfs.health` with `pools`.
fn host(
    listing: &'static [&'static str],
    absent: &'static [(&'static str, &'static str)],
    pools: Value,
) -> impl Fn(&str, &Value) -> Reply + Send + 'static {
    move |method, params| match method {
        "server/discover" => Reply::Result(discover_absent(&[PROTOCOL_VERSION], absent)),
        "tools/list" => Reply::Result(tools(listing)),
        "tools/call" => {
            if params["name"].as_str() == Some(ZFS_TOOL) {
                Reply::Result(json!({"structuredContent": pools.clone()}))
            } else {
                Reply::Error(-32602, "unknown tool")
            }
        }
        _ => Reply::Error(-32601, "no"),
    }
}

async fn gather_from(
    listing: &'static [&'static str],
    absent: &'static [(&'static str, &'static str)],
    pools: Value,
) -> (Snapshot, Vec<Value>) {
    let (rd, wr, seen) = serve(host(listing, absent, pools));
    let mut client = Client::connect(rd, wr, Duration::from_secs(2))
        .await
        .unwrap();
    let snap = gather(&mut client, NOW, MAX_SAMPLING_INTERVAL_S).await;
    let seen = seen.lock().unwrap().clone();
    (snap, seen)
}

#[test]
fn the_pool_state_is_read_from_the_tool_never_re_derived() {
    let pools = parse_zfs(&broken_pools()).unwrap();
    assert_eq!(pools.len(), 1);
    let pool = &pools[0];
    assert_eq!(pool.name, "pool0");
    assert_eq!(pool.state, ZfsState::Suspended);
    assert_eq!(pool.action, "attach the missing device");
    assert_eq!(pool.msg, "One or more devices could not be opened");
    assert_eq!(pool.members.len(), 2);
    // The member that is not ONLINE is named; the one that is, is not.
    let faulted: Vec<&str> = pool
        .faulted_members()
        .iter()
        .map(|m| m.name.as_str())
        .collect();
    assert_eq!(faulted, ["mirror-0", "sdc"]);
    assert!(!pool.is_whole(), "a SUSPENDED pool is not whole");

    let healthy = parse_zfs(&healthy_pools()).unwrap();
    assert!(healthy[0].is_whole(), "every member ONLINE is a whole pool");
    multitop_testassert::assert_empty!(healthy[0].faulted_members());

    // A pool whose own state is ONLINE but whose member is not: not whole.
    // The rule is the estate's (check_zfs_pool's): the pool AND its members.
    let degraded = parse_zfs(&json!({"pools": [
        {"name": "p", "state": "ONLINE", "members": [{"name": "sdb", "state": "FAULTED"}]},
    ]}))
    .unwrap();
    assert!(
        !degraded[0].is_whole(),
        "a FAULTED member is not a whole pool"
    );

    // An answer that is not the tool's shape is an error, never a guess.
    assert_eq!(
        parse_zfs(&json!({})).unwrap_err(),
        "the answer has no `pools` list"
    );
    assert_eq!(
        parse_zfs(&json!({"pools": [{"name": "p", "state": "ONLINE"}]})).unwrap_err(),
        "pool p has no `members` list"
    );
}

#[tokio::test]
async fn a_host_that_serves_the_tool_is_named_and_its_pools_are_shown() {
    // `.33`: the tool is listed, so it is asked, and its answer is the fact.
    let (snap, seen) = gather_from(
        &["health.list", "cron.status", ZFS_TOOL],
        &[],
        healthy_pools(),
    )
    .await;
    assert!(matches!(snap.zfs, Part::Ready(ref p) if p.len() == 1));
    assert!(
        seen.iter()
            .any(|r| r["method"] == "tools/call" && r["params"]["name"] == ZFS_TOOL),
        "the tool was asked: {seen:?}"
    );

    let mut s = snap;
    s.health = Part::Absent("no health suite on this machine".into());
    s.cron = Part::Absent("no jobs".into());
    let f = plain(&render(
        "media-server",
        &OpsState::Ready(Box::new(s)),
        80,
        40,
        NOW,
        &pal(),
    ));
    assert!(
        f.iter().any(|l| l.contains("zfs") && l.contains("pool0")),
        "the view names the section and the pool: {f:?}"
    );
    assert!(
        f.iter().any(|l| l.contains("1 pool whole: pool0")),
        "the pool's state is shown, in the words the other client uses: {f:?}"
    );
}

#[tokio::test]
async fn a_host_without_the_tool_says_why_it_is_absent() {
    // `.158`: no ZFS, so the tool is not listed and the server says why.
    let (snap, seen) = gather_from(
        &["health.list", "cron.status"],
        &[(ZFS_TOOL, NO_ZFS)],
        json!({"pools": []}),
    )
    .await;
    assert_eq!(snap.zfs, Part::Absent(NO_ZFS.into()));
    assert!(
        !seen
            .iter()
            .any(|r| r["method"] == "tools/call" && r["params"]["name"] == ZFS_TOOL),
        "a tool the host does not list is never called: {seen:?}"
    );

    let mut s = snap;
    s.health = Part::Absent("no health suite on this machine".into());
    s.cron = Part::Absent("no jobs".into());
    let f = plain(&render(
        "pihole",
        &OpsState::Ready(Box::new(s)),
        80,
        40,
        NOW,
        &pal(),
    ));
    assert!(
        f.iter().any(|l| l.contains("zfs") && l.contains(NO_ZFS)),
        "the view names the tool and the server's reason: {f:?}"
    );
    assert!(
        !f.iter().any(|l| l.contains("✓ zfs")),
        "an absence is never a green line: {f:?}"
    );
}

#[tokio::test]
async fn a_pool_that_is_not_whole_is_visible_not_merely_listed() {
    let (snap, _) = gather_from(&[ZFS_TOOL], &[], broken_pools()).await;
    let mut s = snap;
    s.health = Part::Absent("no health suite on this machine".into());
    s.cron = Part::Absent("no jobs".into());
    let f = plain(&render(
        "storage",
        &OpsState::Ready(Box::new(s)),
        80,
        40,
        NOW,
        &pal(),
    ));
    // The pool's name, its state, and the member that is not ONLINE.
    assert!(
        f.iter()
            .any(|l| l.contains("pool0") && l.contains("SUSPENDED") && l.contains("sdc")),
        "the pool that is not whole is named, with its member: {f:?}"
    );
    assert!(
        f.iter()
            .any(|l| l.contains("ZFS says: attach the missing device")),
        "what ZFS says to do about it is shown: {f:?}"
    );
    assert!(
        f.iter().any(|l| l.starts_with("✗ zfs")),
        "a pool that is not whole is a fault, not a quiet line: {f:?}"
    );
}

#[tokio::test]
async fn an_answer_that_cannot_be_read_says_so_and_hides_nothing() {
    let (rd, wr, _) = serve(|method, _| match method {
        "server/discover" => Reply::Result(discover_absent(&[PROTOCOL_VERSION], &[])),
        "tools/list" => Reply::Result(tools(&[ZFS_TOOL])),
        "tools/call" => Reply::Error(-32603, "zpool status failed"),
        _ => Reply::Error(-32601, "no"),
    });
    let mut client = Client::connect(rd, wr, Duration::from_secs(2))
        .await
        .unwrap();
    let snap = gather(&mut client, NOW, MAX_SAMPLING_INTERVAL_S).await;
    assert_eq!(
        snap.zfs,
        Part::Failed("zpool status failed (-32603)".into())
    );
    let f = plain(&render(
        "storage",
        &OpsState::Ready(Box::new(snap)),
        80,
        40,
        NOW,
        &pal(),
    ));
    assert!(
        f.iter()
            .any(|l| l.contains("⚠ zfs") && l.contains("could not read")),
        "{f:?}"
    );
}

#[test]
fn the_pool_names_are_searchable() {
    let s = Snapshot {
        server: ("mcp_host".into(), "1.65.2".into()),
        at: NOW,
        sampled_every_s: MAX_SAMPLING_INTERVAL_S,
        health: Part::Absent("none".into()),
        cron: Part::Absent("none".into()),
        containers: Part::Absent("none".into()),
        zfs: Part::Ready(parse_zfs(&broken_pools()).unwrap()),
        vpn: Part::Absent("none".into()),
        disk: Part::Absent("none".into()),
        facts: Part::Absent("none".into()),
        alerts: Part::Absent("none".into()),
    };
    let terms = s.terms();
    assert!(terms.contains(&"pool0"), "{terms:?}");
    assert!(terms.contains(&"sdc"), "{terms:?}");
}
