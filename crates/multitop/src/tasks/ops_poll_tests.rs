use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use tokio::sync::{mpsc, watch};

use super::{poll, Target};
use crate::app::Msg;
use crate::mcp::client::{Client, PROTOCOL_VERSION};
use crate::mcp::fake::{discover, serve, tools, Reply};
use crate::ops::cache::{Cache, Raw, RawSnapshot};
use crate::ops::{OpsState, Part};

const NOW: i64 = 1_790_170_000;

/// A host that answers its first `live` requests, then hangs up.
fn host(live: usize) -> impl Fn(&str, &serde_json::Value) -> Reply + Send + 'static {
    let seen = Arc::new(AtomicUsize::new(0));
    move |method, params| {
        if seen.fetch_add(1, Ordering::SeqCst) >= live {
            return Reply::HangUp;
        }
        match method {
            "server/discover" => Reply::Result(discover(&[PROTOCOL_VERSION])),
            "tools/list" => Reply::Result(tools(&["cron.status"])),
            "tools/call" if params["name"] == "cron.status" => {
                Reply::Result(json!({"structuredContent": {"jobs": []}}))
            }
            _ => Reply::Error(-32601, "no"),
        }
    }
}

/// The cache the poll keeps its answers in: a directory of its own.
fn cache(dir: &tempfile::TempDir) -> Cache {
    Cache::beside(&dir.path().join("config.toml"))
}

/// An answer, as a gather would have left it.
fn answer(at: i64) -> RawSnapshot {
    RawSnapshot {
        server: ("mcp_host".into(), "1.0.0".into()),
        at,
        sampled_every_s: 450,
        health: Raw::Absent("this host lists no health checks".into()),
        health_list: Raw::Absent("this host lists no health checks".into()),
        vpn: Raw::Absent("this host lists no net.egress".into()),
        cron: Raw::Value(json!({"jobs": []})),
        containers: Raw::Absent("no Docker on this host".into()),
        zfs: Raw::Absent("this host lists no zfs.health".into()),
        disk: Raw::Absent("this host lists no disk.usage".into()),
        facts: Raw::Absent("this host lists no host.facts".into()),
        alerts: Raw::Absent("this host lists no alerts.since".into()),
    }
}

/// The key the tests file their answers under.
const KEY: &str = "ztomer@192.168.0.33:22";

#[tokio::test]
async fn a_broken_session_says_why_under_the_last_answer_then_reconnects() {
    let (tx, mut rx) = mpsc::channel(8);
    let dims = Arc::new(watch::channel((80, 24)).1);
    let dir = tempfile::tempdir().unwrap();
    let to = Target {
        panel: 1,
        gen: 7,
        tx,
        dims,
        cache: cache(&dir),
        key: KEY.into(),
    };
    let opened = Arc::new(AtomicUsize::new(0));
    let count = Arc::clone(&opened);
    // First session: discover + tools/list + one poll (1 call), then gone.
    // Every later open is refused.
    let connect = move || {
        let n = count.fetch_add(1, Ordering::SeqCst);
        async move {
            if n == 0 {
                let (r, w, _) = serve(host(3));
                Client::connect(r, w, Duration::from_secs(2))
                    .await
                    .map(Box::new)
                    .map_err(|e| e.to_string())
            } else {
                Err("ssh: connect to host h port 22: Connection refused".to_string())
            }
        }
    };
    let task = tokio::spawn(poll(
        to,
        connect,
        Duration::from_millis(5),
        Duration::from_millis(5),
        || NOW,
    ));
    let mut states = Vec::new();
    for _ in 0..3 {
        match rx.recv().await {
            Some(Msg::Ops {
                panel,
                gen,
                state,
                dims,
            }) => {
                assert_eq!((panel, gen, dims), (1, 7, (80, 24)));
                states.push(*state);
            }
            other => panic!("{other:?}"),
        }
    }
    task.abort();
    let OpsState::Ready(first) = &states[0] else {
        panic!("{:?}", states[0])
    };
    assert_eq!(first.cron, Part::Ready(vec![]));
    assert!(matches!(first.containers, Part::Absent(_)));
    match &states[1] {
        OpsState::Failed {
            why,
            last: Some(last),
        } => {
            assert!(why.starts_with("mcp_host closed the session"), "{why}");
            assert_eq!(last, first, "the last answer is kept, labelled as such");
        }
        other => panic!("{other:?}"),
    }
    match &states[2] {
        OpsState::Failed { why, last: Some(_) } => {
            assert!(why.ends_with("Connection refused"), "{why}");
        }
        other => panic!("{other:?}"),
    }
    assert!(
        opened.load(Ordering::SeqCst) >= 2,
        "a broken session is reopened, not reused"
    );
}

#[tokio::test]
async fn the_poll_ends_when_nobody_listens() {
    let (tx, rx) = mpsc::channel(1);
    drop(rx);
    let dir = tempfile::tempdir().unwrap();
    let to = Target {
        panel: 0,
        gen: 0,
        tx,
        dims: Arc::new(watch::channel((1, 1)).1),
        cache: cache(&dir),
        key: KEY.into(),
    };
    let connect =
        || async { Err::<Box<Client<tokio::io::Empty, tokio::io::Sink>>, _>("no".to_string()) };
    tokio::time::timeout(
        Duration::from_secs(2),
        poll(
            to,
            connect,
            Duration::from_millis(5),
            Duration::from_millis(5),
            || NOW,
        ),
    )
    .await
    .expect("a poll with no listener must return");
}

/// **THE ESTATE IS NOT ASKED FOR AN ANSWER THE CLIENT ALREADY HAS.** A
/// fresh answer in the cache - the pass's, or this poll's own - is shown,
/// and no session is opened at all: one card, one read of the estate.
#[tokio::test]
async fn a_fresh_answer_is_shown_without_asking_the_estate() {
    let (tx, mut rx) = mpsc::channel(8);
    let dims = Arc::new(watch::channel((80, 24)).1);
    let dir = tempfile::tempdir().unwrap();
    let kept = cache(&dir);
    kept.write(KEY, &answer(NOW - 10)).unwrap();
    let to = Target {
        panel: 0,
        gen: 0,
        tx,
        dims,
        cache: kept,
        key: KEY.into(),
    };
    let opened = Arc::new(AtomicUsize::new(0));
    let count = Arc::clone(&opened);
    let connect = move || {
        count.fetch_add(1, Ordering::SeqCst);
        async {
            Err::<Box<Client<tokio::io::Empty, tokio::io::Sink>>, _>(
                "the estate was asked".to_string(),
            )
        }
    };
    let task = tokio::spawn(poll(
        to,
        connect,
        Duration::from_secs(450),
        Duration::from_millis(5),
        || NOW,
    ));
    let state = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("no answer")
        .expect("no answer");
    task.abort();
    let Msg::Ops { state, .. } = state else {
        panic!("not an Ops message")
    };
    let OpsState::Ready(snap) = *state else {
        panic!("the cached answer was not shown")
    };
    assert_eq!(snap.at, NOW - 10, "the answer shown is the cached one");
    assert_eq!(
        opened.load(Ordering::SeqCst),
        0,
        "the estate was asked for an answer the client already had"
    );
}
