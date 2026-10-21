//! The client's TOOL LIST: where it comes from, and what it may hide.
//!
//! servers ROADMAP E61: the list is the biggest thing this client pulls off
//! a host, and it changes at a RELEASE - so it is kept between sessions,
//! keyed to what the server says about the list it serves.
//!
//! servers ROADMAP E72: the key is the server's OWN WORD - the digest of
//! the exact list `tools/list` answers, published in `server/discover`.
//! The version and the absent set were a GUESS about the list, and the
//! guess was wrong often enough to measure: it re-listed on every release,
//! including the four of the last ten that touched no tool table at all,
//! and it could not see a table that moved without a release. The digest
//! half of that - and what the client does when the server publishes no
//! digest at all - is proved in `catalogue_digest_tests.rs`.
//!
//! What is proved here, and each is a way the kept list could lie:
//!
//! - the kept list is the WHOLE list: a tool the cache omits may not
//!   vanish from the panel, and the cache is an optimisation, never a
//!   filter;
//! - a RELEASE asks the server again, when the server publishes no digest;
//! - a host that gained or lost a tool asks the server again, when the
//!   server publishes no digest;
//! - a kept list past the window the server gave is not used at all, and
//!   the client says why;
//! - a server that names no version keeps nothing at all.

use std::time::Duration;

use serde_json::Value;

use super::catalogue::{Catalogue, Miss, Store};
use super::client::{Client, PROTOCOL_VERSION};
use super::fake::{discover, discover_absent_as, discover_as, serve, tools_cached, Reply};

const T: Duration = Duration::from_secs(2);
/// The window the estate's servers give a list: an hour, in milliseconds.
const TTL_MS: u64 = 3_600_000;
/// The tools the fake's server offers.
const TOOLS: [&str; 3] = ["cron.status", "health.list", "alerts.since"];

/// A store in a directory of its own.
fn store(dir: &tempfile::TempDir) -> Store {
    Store::at(Some(dir.path().join("catalogue.json")))
}

/// The fake's server: this version, this tool set, this window.
fn server(version: &str) -> impl Fn(&str, &Value) -> Reply + Send + 'static {
    let version = version.to_string();
    move |method, _| match method {
        "server/discover" => Reply::Result(discover_as(&[PROTOCOL_VERSION], "mcp_host", &version)),
        "tools/list" => Reply::Result(tools_cached(&TOOLS, TTL_MS)),
        _ => Reply::Error(-32601, "no"),
    }
}

/// The methods the fake saw, in order.
fn methods(seen: &super::fake::Seen) -> Vec<String> {
    seen.lock()
        .unwrap()
        .iter()
        .map(|r| r["method"].as_str().unwrap_or_default().to_string())
        .collect()
}

/// **THE KEPT LIST IS THE WHOLE LIST, AND THE SERVER IS NOT ASKED AGAIN.**
/// The cache is an optimisation, never a filter: a tool the kept list
/// omitted would lose its section from the panel, and the second session
/// must know every tool the first one knew.
#[tokio::test]
async fn the_kept_list_is_the_whole_list_and_the_server_is_not_asked_again() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(&dir);
    let (r, w, seen) = serve(server("1.99.19"));
    let first = Client::connect_with(r, w, T, &store).await.unwrap();
    assert_eq!(first.tools, TOOLS);
    assert!(
        matches!(first.catalogue, Catalogue::Listed { .. }),
        "the first session has nothing to keep: {:?}",
        first.catalogue
    );
    assert_eq!(methods(&seen), ["server/discover", "tools/list"]);

    let (r, w, seen2) = serve(server("1.99.19"));
    let second = Client::connect_with(r, w, T, &store).await.unwrap();
    assert_eq!(
        methods(&seen2),
        ["server/discover"],
        "the kept list was asked for again"
    );
    assert_eq!(
        second.tools, first.tools,
        "a tool the kept list omitted vanished from the panel"
    );
    assert!(
        matches!(second.catalogue, Catalogue::Kept { .. }),
        "the second session does not say where its list came from: {:?}",
        second.catalogue
    );
}

/// **THE PANEL KNOWS WHICH LIST IT DREW.** The state says whether the list
/// came from the server or from the kept copy, and how old the kept copy
/// is: a list nobody can date is a shape nobody can trust.
#[tokio::test]
async fn the_client_says_where_the_list_came_from() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(&dir);
    let (r, w, _) = serve(server("1.99.19"));
    let _ = Client::connect_with(r, w, T, &store).await.unwrap();
    let (r, w, _) = serve(server("1.99.19"));
    let second = Client::connect_with(r, w, T, &store).await.unwrap();
    match second.catalogue {
        Catalogue::Kept { age_s, ttl_s, .. } => {
            assert!(age_s >= 0, "the age is the truth: {age_s}");
            assert_eq!(ttl_s, 3_600, "the window is the server's own");
        }
        other => panic!("the kept list was not used: {other:?}"),
    }
}

/// **A RELEASE ASKS THE SERVER AGAIN.** The version moved, so the kept list
/// is for the old binary and cannot be used: the tool the new binary
/// added must be in the panel.
#[tokio::test]
async fn a_release_asks_the_server_again() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(&dir);
    let (r, w, _) = serve(server("1.99.19"));
    let _ = Client::connect_with(r, w, T, &store).await.unwrap();

    let (r, w, seen) = serve(|method, _| match method {
        "server/discover" => Reply::Result(discover_as(&[PROTOCOL_VERSION], "mcp_host", "1.99.20")),
        "tools/list" => Reply::Result(tools_cached(
            &[
                "cron.status",
                "health.list",
                "alerts.since",
                "qbit.torrents",
            ],
            TTL_MS,
        )),
        _ => Reply::Error(-32601, "no"),
    });
    let after = Client::connect_with(r, w, T, &store).await.unwrap();
    assert_eq!(
        methods(&seen),
        ["server/discover", "tools/list"],
        "a release did not ask the server for the list again"
    );
    assert!(
        after.has_tool("qbit.torrents"),
        "the tool the new binary added is not in the panel"
    );
    match after.catalogue {
        Catalogue::Relisted {
            miss: Miss::Released { kept, now },
            ..
        } => {
            assert_eq!(kept, "1.99.19");
            assert_eq!(now, "1.99.20");
        }
        other => panic!("a release is not named: {other:?}"),
    }
}

/// **A HOST THAT GAINED OR LOST A TOOL ASKS THE SERVER AGAIN.** The
/// absent set moved, so the kept list is not the list this host is
/// offering now.
#[tokio::test]
async fn a_changed_tool_set_asks_the_server_again() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(&dir);
    let (r, w, _) = serve(server("1.99.19"));
    let _ = Client::connect_with(r, w, T, &store).await.unwrap();

    let (r, w, seen) = serve(|method, _| match method {
        "server/discover" => Reply::Result(discover_absent_as(
            &[PROTOCOL_VERSION],
            &[("zfs.health", "no ZFS on this host")],
            "mcp_host",
            "1.99.19",
        )),
        "tools/list" => Reply::Result(tools_cached(&TOOLS, TTL_MS)),
        _ => Reply::Error(-32601, "no"),
    });
    let after = Client::connect_with(r, w, T, &store).await.unwrap();
    assert_eq!(
        methods(&seen),
        ["server/discover", "tools/list"],
        "a changed tool set did not ask the server for the list again"
    );
    assert!(matches!(
        after.catalogue,
        Catalogue::Relisted {
            miss: Miss::Reshaped { .. },
            ..
        }
    ));
}

/// **A KEPT LIST PAST ITS WINDOW IS NOT USED.** The server's own `ttlMs`
/// bounds the kept list, and a list that outlived it would be caching
/// something the server never promised.
#[tokio::test]
async fn a_kept_list_past_its_window_is_not_used() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(&dir);
    let (r, w, _) = serve(server("1.99.19"));
    let _ = Client::connect_with(r, w, T, &store).await.unwrap();

    // The kept list is aged past the server's window: it was taken two
    // hours ago, and the server allows an hour.
    let path = dir.path().join("catalogue.json");
    let entry: super::catalogue::Entry =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let aged = super::catalogue::Entry {
        fetched_at: entry.fetched_at - 7_200,
        ..entry
    };
    std::fs::write(&path, serde_json::to_string(&aged).unwrap()).unwrap();

    let (r, w, seen) = serve(server("1.99.19"));
    let after = Client::connect_with(r, w, T, &store).await.unwrap();
    assert_eq!(
        methods(&seen),
        ["server/discover", "tools/list"],
        "a list past its window was used"
    );
    match after.catalogue {
        Catalogue::Relisted {
            miss: Miss::Expired { age_s, ttl_s },
            ..
        } => {
            assert!(age_s >= 7_200, "the age is the truth: {age_s}");
            assert_eq!(ttl_s, 3_600);
        }
        other => panic!("an expired list is not named: {other:?}"),
    }
}

/// **A LIST OF SEVERAL PAGES IS KEPT WHOLE.** The list is followed to its
/// last page; a kept list that held only the first page would lose every
/// tool on the second, and a tool that is not in the list has no section.
#[tokio::test]
async fn a_list_of_several_pages_is_kept_whole() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(&dir);
    let (r, w, _) = serve(|method, params| match method {
        "server/discover" => Reply::Result(discover(&[PROTOCOL_VERSION])),
        "tools/list" if params.get("cursor").is_none() => {
            let mut page = tools_cached(&["cron.status"], TTL_MS);
            page["nextCursor"] = serde_json::json!("p2");
            Reply::Result(page)
        }
        "tools/list" => Reply::Result(tools_cached(&["health.list", "alerts.since"], TTL_MS)),
        _ => Reply::Error(-32601, "no"),
    });
    let first = Client::connect_with(r, w, T, &store).await.unwrap();
    assert_eq!(first.tools, ["cron.status", "health.list", "alerts.since"]);

    let (r, w, _) = serve(|method, _| match method {
        "server/discover" => Reply::Result(discover(&[PROTOCOL_VERSION])),
        "tools/list" => Reply::Error(-32601, "the whole list was asked for again"),
        _ => Reply::Error(-32601, "no"),
    });
    let second = Client::connect_with(r, w, T, &store).await.unwrap();
    assert_eq!(
        second.tools, first.tools,
        "a tool on the second page vanished from the panel"
    );
}

/// **A SERVER THAT NAMES NO VERSION KEEPS NOTHING.** The version is the
/// only thing that says which binary holds the tool table, so without it
/// there is no key, and a list that could not be keyed is a list that is
/// asked for every time.
#[tokio::test]
async fn a_server_that_names_no_version_keeps_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(&dir);
    let (r, w, _) = serve(|method, _| match method {
        "server/discover" => Reply::Result(serde_json::json!({
            "supportedVersions": [PROTOCOL_VERSION],
            "capabilities": {},
        })),
        "tools/list" => Reply::Result(tools_cached(&TOOLS, TTL_MS)),
        _ => Reply::Error(-32601, "no"),
    });
    let _ = Client::connect_with(r, w, T, &store).await.unwrap();
    assert!(
        !dir.path().join("catalogue.json").exists(),
        "a list with no key was kept"
    );
    let (r, w, seen) = serve(|method, _| match method {
        "server/discover" => Reply::Result(serde_json::json!({
            "supportedVersions": [PROTOCOL_VERSION],
            "capabilities": {},
        })),
        "tools/list" => Reply::Result(tools_cached(&TOOLS, TTL_MS)),
        _ => Reply::Error(-32601, "no"),
    });
    let _ = Client::connect_with(r, w, T, &store).await.unwrap();
    assert_eq!(
        methods(&seen),
        ["server/discover", "tools/list"],
        "a list with no key was kept and used"
    );
}
