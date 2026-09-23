use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use tokio::sync::{mpsc, watch};

use super::{poll, Target};
use crate::app::Msg;
use crate::mcp::client::{Client, PROTOCOL_VERSION};
use crate::mcp::fake::{discover, serve, tools, Reply};
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

#[tokio::test]
async fn a_broken_session_says_why_under_the_last_answer_then_reconnects() {
    let (tx, mut rx) = mpsc::channel(8);
    let dims = Arc::new(watch::channel((80, 24)).1);
    let to = Target {
        panel: 1,
        gen: 7,
        tx,
        dims,
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
    let task = tokio::spawn(poll(to, connect, Duration::from_millis(5), || NOW));
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
    let to = Target {
        panel: 0,
        gen: 0,
        tx,
        dims: Arc::new(watch::channel((1, 1)).1),
    };
    let connect =
        || async { Err::<Box<Client<tokio::io::Empty, tokio::io::Sink>>, _>("no".to_string()) };
    tokio::time::timeout(
        Duration::from_secs(2),
        poll(to, connect, Duration::from_millis(5), || NOW),
    )
    .await
    .expect("a poll with no listener must return");
}
