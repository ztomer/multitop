//! The AGENT: the poll in a loop, paced by the cadence the estate
//! publishes.
//!
//! servers ROADMAP E53: the installed agent's tick was a number this repo
//! carried, and launchd fixes an interval when the job is loaded - so the
//! tick could not follow the estate. These tests pin the three things that
//! has to mean now:
//!
//! - the agent WAITS the tick the published cadence implies, and a
//!   different cadence gives a different wait;
//! - the agent SAYS where the cadence came from;
//! - a cadence the agent could not read is STATED, and the rate the client
//!   was keeping is kept - never silently a number this repo carries.

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicI64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{DuplexStream, ReadHalf, WriteHalf};

use super::*;
use crate::config::Server;
use crate::mcp::client::{Client, PROTOCOL_VERSION};
use crate::mcp::fake::{discover, serve, tools, Reply};
use crate::ops::cache::Cache;

/// The epoch second the agent's clock reads.
const NOW: i64 = 1_790_170_000;

/// The clock the agent reads in the loop test: the test moves it, so a
/// cycle's answer is old enough to be due again.
static CLOCK: AtomicI64 = AtomicI64::new(NOW);

/// The clock the agent reads.
fn now() -> i64 {
    CLOCK.load(Ordering::SeqCst)
}

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

/// The verdict a host publishes, with the cadence it published beside it.
fn verdict(cadence_s: Option<i64>) -> Value {
    let mut verdict = json!({"checks": [], "oldest_age": "10 min ago"});
    if let Some(cadence_s) = cadence_s {
        verdict["cadence_s"] = json!(cadence_s);
        verdict["cadence"] = json!("every 15 min");
    }
    verdict
}

/// An opener that answers from an in-memory `mcp_host` publishing a
/// cadence, and counts the sessions opened against it.
fn opener(cadence_s: Option<i64>, opened: Arc<AtomicUsize>) -> impl FnMut(&Server) -> Opened {
    move |_: &Server| {
        let opened = Arc::clone(&opened);
        let verdict = verdict(cadence_s);
        Box::pin(async move {
            opened.fetch_add(1, Ordering::SeqCst);
            let (r, w, _) = serve(move |method, params| match method {
                "server/discover" => Reply::Result(discover(&[PROTOCOL_VERSION])),
                "tools/list" => Reply::Result(tools(&["health.list"])),
                "resources/read" => {
                    Reply::Result(json!({"contents": [{"text": verdict.to_string()}]}))
                }
                "tools/call" => match params["name"].as_str().unwrap_or_default() {
                    "health.list" => Reply::Result(json!({"structuredContent": {"checks": []}})),
                    _ => Reply::Error(-32602, "unknown tool"),
                },
                _ => Reply::Error(-32601, "no"),
            });
            Client::connect(r, w, Duration::from_secs(2))
                .await
                .map(|c| Box::new(c) as Fake)
                .map_err(|e| e.to_string())
        })
    }
}

/// A cache in a directory of its own.
fn cache(dir: &tempfile::TempDir) -> Cache {
    Cache::beside(&dir.path().join("config.toml"))
}

/// **THE AGENT WAITS THE TICK THE PUBLISHED CADENCE IMPLIES.** A cadence of
/// 300 is a rate of 150; a cadence of 900 is a rate of 450. The agent waits
/// the one, then the other - and the number it waits is not in this repo.
#[tokio::test]
async fn the_agent_waits_the_tick_the_published_cadence_implies() {
    let dir = tempfile::tempdir().unwrap();
    let first = cache(&dir);
    let servers = vec![server("192.168.0.33")];

    let mut fast = opener(Some(300), Arc::new(AtomicUsize::new(0)));
    let quick = cycle(&servers, &first, now, &mut fast).await;
    assert_eq!(quick.tick.secs, 150 - crate::ops::tick::TICK_MARGIN_S);
    assert_eq!(
        quick.wait.as_secs(),
        u64::try_from(quick.tick.secs).unwrap(),
        "the pass took no time"
    );

    // A cache of its own: the first cycle's answer is in the first cache,
    // and a host whose answer is fresh is not asked again - so the second
    // cycle has to read a cadence of its own.
    let other = tempfile::tempdir().unwrap();
    let mut slow = opener(Some(900), Arc::new(AtomicUsize::new(0)));
    let patient = cycle(&servers, &cache(&other), now, &mut slow).await;
    assert_eq!(patient.tick.secs, 450 - crate::ops::tick::TICK_MARGIN_S);
    assert!(
        patient.wait > quick.wait,
        "a slower publication did not make the agent wait longer"
    );
}

/// **THE AGENT SAYS WHERE THE CADENCE CAME FROM.** A tick in a log that
/// names nothing cannot be checked.
#[tokio::test]
async fn the_agent_says_where_the_cadence_came_from() {
    let dir = tempfile::tempdir().unwrap();
    let cache = cache(&dir);
    let servers = vec![server("192.168.0.33")];
    let mut open = opener(Some(900), Arc::new(AtomicUsize::new(0)));
    let cycle = cycle(&servers, &cache, now, &mut open).await;
    let said = lines(&cycle, now()).join("\n");
    assert!(
        said.contains("every 15 min"),
        "the agent does not say what the estate publishes: {said}"
    );
    assert!(
        said.contains("health://latest"),
        "the agent does not say where the cadence was read: {said}"
    );
    assert!(
        said.contains("192.168.0.33"),
        "the agent does not name the host: {said}"
    );
    assert!(
        said.contains("gathered"),
        "the agent does not say what the pass found: {said}"
    );
}

/// **A CADENCE THE AGENT COULD NOT READ IS STATED, AND THE RATE THE CLIENT
/// WAS KEEPING IS KEPT.** A host that publishes no cadence is not a host
/// with no rate, and it is not a reason to fall back on a number this repo
/// carries and call it the published rate.
#[tokio::test]
async fn a_cadence_the_agent_could_not_read_is_stated() {
    let dir = tempfile::tempdir().unwrap();
    let cache = cache(&dir);
    let servers = vec![server("192.168.0.33")];
    let mut open = opener(None, Arc::new(AtomicUsize::new(0)));
    let cycle = cycle(&servers, &cache, now, &mut open).await;
    assert_eq!(
        cycle.tick.secs,
        crate::ops::rate::FALLBACK_INTERVAL_S - crate::ops::tick::TICK_MARGIN_S,
        "the agent kept the client's fallback rate"
    );
    let said = lines(&cycle, now()).join("\n");
    assert!(
        said.contains("the cadence could not be read"),
        "the agent does not say the cadence could not be read: {said}"
    );
    assert!(
        said.contains("keeps its rate"),
        "the agent does not say what it keeps instead: {said}"
    );
}

/// **THE AGENT PASSES AGAIN ONLY AFTER THE TICK.** The loop waits the tick
/// the cadence implies: not before it, and not because a number in this
/// repo said so.
#[tokio::test(start_paused = true)]
async fn the_agent_passes_again_only_after_the_tick() {
    CLOCK.store(NOW, Ordering::SeqCst);
    let dir = tempfile::tempdir().unwrap();
    let cache = cache(&dir);
    let servers = vec![server("192.168.0.33")];
    let opened = Arc::new(AtomicUsize::new(0));
    let counter = Arc::clone(&opened);
    let agent = tokio::spawn(async move {
        let mut open = opener(Some(300), counter);
        run(&servers, &cache, now, &mut open).await;
    });
    // The first pass runs at once.
    tokio::task::yield_now().await;
    tokio::task::yield_now().await;
    assert_eq!(opened.load(Ordering::SeqCst), 1, "the first pass");

    // The answer is old enough to be due again, but the tick has not come.
    CLOCK.store(NOW + 1_000, Ordering::SeqCst);
    tokio::time::advance(Duration::from_secs(60)).await;
    tokio::task::yield_now().await;
    tokio::task::yield_now().await;
    assert_eq!(
        opened.load(Ordering::SeqCst),
        1,
        "the agent passed before its tick: the tick is not pacing it"
    );

    // Past the tick: the agent passes again.
    tokio::time::advance(Duration::from_secs(200)).await;
    tokio::task::yield_now().await;
    tokio::task::yield_now().await;
    assert_eq!(
        opened.load(Ordering::SeqCst),
        2,
        "the agent did not pass again after its tick"
    );
    agent.abort();
}

/// **THE AGENT PICKS UP A REPLACED BINARY.** The agent is a long-lived
/// process - that is what lets it pace itself - so an upgraded binary would
/// otherwise leave it running the old one, and the old one keeps the old
/// rule. It hands over to the new binary instead.
#[test]
fn the_agent_picks_up_a_replaced_binary() {
    let dir = tempfile::tempdir().unwrap();
    let binary = dir.path().join("multitop");
    std::fs::write(&binary, b"the old one").unwrap();
    let started = std::fs::metadata(&binary).unwrap().modified().unwrap();

    assert!(
        !replaced(&binary, Some(started)),
        "the binary did not change, and the agent thought it had"
    );
    assert!(
        !replaced(&binary, None),
        "the agent had nothing to compare against, and guessed"
    );

    // The binary is replaced: a new file, with a new moment.
    std::thread::sleep(std::time::Duration::from_millis(1_100));
    std::fs::write(&binary, b"the new one, a longer file").unwrap();
    assert!(
        replaced(&binary, Some(started)),
        "the agent did not notice the binary was replaced"
    );
}
