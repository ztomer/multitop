//! ONE PASS of the Ops poll: the job that has to actually run.
//!
//! servers ROADMAP E36: the poll was bound to the Ops view, so it ran only
//! while a human was watching, and the estate's fingerprint tools fired 0.3
//! a day. These tests are about the JOB: that it gathers what is due, that
//! it does NOT ask the estate for an answer it already has, and that a
//! host it could not reach says so instead of counting as done.

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde_json::json;
use tokio::io::{DuplexStream, ReadHalf, WriteHalf};

use super::pass::*;
use crate::config::Server;
use crate::mcp::client::{Client, PROTOCOL_VERSION};
use crate::mcp::fake::{discover, serve, tools, Reply};
use crate::ops::cache::Cache;

/// The poll's interval, in seconds: the rule.
const EVERY: u64 = 450;
/// The epoch second the pass starts at.
const NOW: i64 = 1_790_170_000;

/// The client the in-memory server is driven through.
type Fake = Box<Client<ReadHalf<DuplexStream>, WriteHalf<DuplexStream>>>;

/// A future a pass can be given.
type Opened = Pin<Box<dyn Future<Output = Result<Fake, String>> + Send>>;

/// A host in the config, with an `mcp` command.
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

/// An opener that answers from an in-memory `mcp_host`, and counts the
/// sessions opened against it.
fn opener(opened: Arc<AtomicUsize>) -> impl FnMut(&Server) -> Opened {
    move |_: &Server| {
        let opened = Arc::clone(&opened);
        Box::pin(async move {
            opened.fetch_add(1, Ordering::SeqCst);
            let (r, w, _) = serve(|method, params| match method {
                "server/discover" => Reply::Result(discover(&[PROTOCOL_VERSION])),
                "tools/list" => Reply::Result(tools(&["cron.status"])),
                "tools/call" if params["name"] == "cron.status" => {
                    Reply::Result(json!({"structuredContent": {"jobs": []}}))
                }
                _ => Reply::Error(-32601, "no"),
            });
            Client::connect(r, w, Duration::from_secs(2))
                .await
                .map(|c| Box::new(c) as Fake)
                .map_err(|e| e.to_string())
        })
    }
}

/// A pass gathers the host that has no answer, and keeps what it found.
#[tokio::test]
async fn a_pass_gathers_what_is_due_and_keeps_it() {
    let dir = tempfile::tempdir().unwrap();
    let cache = Cache::beside(&dir.path().join("config.toml"));
    let opened = Arc::new(AtomicUsize::new(0));
    let servers = vec![server("192.168.0.33")];
    let pass = run(
        &servers,
        &cache,
        NOW,
        Duration::from_secs(EVERY),
        opener(Arc::clone(&opened)),
    )
    .await;
    assert!(pass.complete(), "the pass did not finish: {pass:?}");
    assert_eq!(pass.gathered().count(), 1, "the host was not gathered");
    assert!(
        cache.read("ztomer@192.168.0.33:22").is_some(),
        "the answer was not kept"
    );
}

/// **THE ESTATE IS NOT ASKED TWICE FOR ONE CARD.** A second pass inside the
/// poll's interval finds the answer the first pass kept, and leaves the
/// host alone: the client's second reader costs the estate one read, not
/// two.
#[tokio::test]
async fn a_pass_leaves_a_host_alone_when_its_answer_is_fresh() {
    let dir = tempfile::tempdir().unwrap();
    let cache = Cache::beside(&dir.path().join("config.toml"));
    let opened = Arc::new(AtomicUsize::new(0));
    let servers = vec![server("192.168.0.33")];
    let first = run(
        &servers,
        &cache,
        NOW,
        Duration::from_secs(EVERY),
        opener(Arc::clone(&opened)),
    )
    .await;
    assert_eq!(first.gathered().count(), 1);
    let second = run(
        &servers,
        &cache,
        NOW + 1,
        Duration::from_secs(EVERY),
        opener(Arc::clone(&opened)),
    )
    .await;
    assert_eq!(
        second.gathered().count(),
        0,
        "the estate was asked again for an answer it already gave"
    );
    assert!(second.hosts[0].fresh, "{second:?}");
    assert_eq!(
        opened.load(Ordering::SeqCst),
        1,
        "the estate was asked twice for one card"
    );
}

/// A host the pass could not reach says so, and the pass is NOT complete:
/// a poll that silently does nothing is the defect this round is about.
#[tokio::test]
async fn a_host_the_pass_could_not_reach_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let cache = Cache::beside(&dir.path().join("config.toml"));
    let servers = vec![server("192.168.0.33")];
    let pass = run(
        &servers,
        &cache,
        NOW,
        Duration::from_secs(EVERY),
        |_: &Server| {
            Box::pin(async {
                Err::<Fake, _>("ssh: connect to host 192.168.0.33 port 22: refused".to_string())
            }) as Opened
        },
    )
    .await;
    assert!(!pass.complete(), "a pass that reached nobody is complete");
    let why = pass.hosts[0].why.as_deref().unwrap_or_default();
    assert!(
        why.contains("refused"),
        "the reason is not the server's: {why}"
    );
}
