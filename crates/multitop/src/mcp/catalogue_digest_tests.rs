//! The client's TOOL LIST, checked against the server's OWN WORD: the
//! DIGEST of the list `tools/list` answers.
//!
//! servers ROADMAP E72: the kept list used to be checked by a GUESS - the
//! server's advertised version and the set of tools it says it does not
//! offer. The guess was wrong often enough to measure: it re-listed on
//! every release, including the four of the last ten that touched no tool
//! table at all, and it could not see a table that moved WITHOUT a
//! release. The server now publishes the digest of the exact list it
//! serves, and the client checks its kept copy against that word.
//!
//! What is proved here, and each is a way the kept list could lie:
//!
//! - a digest that CHANGED puts the kept list aside, even when the
//!   version and the absent set did not move;
//! - a digest that did NOT change keeps the kept list, even across a
//!   release - and the host is not asked again;
//! - the digest the server handed is KEPT, because the client cannot
//!   compute it: it covers the schemas too;
//! - a digest the client CANNOT CHECK - none published, or one that is not
//!   a digest - says the fact is ABSENT, and never means "the copy is
//!   fine", and it does not make the client re-list every session;
//! - a kept list taken before the server published a digest is put aside
//!   ONCE, and then the digest is remembered beside it.

use std::time::Duration;

use serde_json::Value;

use super::catalogue::{Catalogue, Miss, Proof, Store};
use super::client::{Client, PROTOCOL_VERSION};
use super::fake::{discover_digest_as, serve, tools_cached, Reply, DIGEST, OTHER_DIGEST};

const T: Duration = Duration::from_secs(2);
/// The window the estate's servers give a list: an hour, in milliseconds.
const TTL_MS: u64 = 3_600_000;
/// The tools the fake's server offers.
const TOOLS: [&str; 3] = ["cron.status", "health.list", "alerts.since"];

/// A store in a directory of its own.
fn store(dir: &tempfile::TempDir) -> Store {
    Store::at(Some(dir.path().join("catalogue.json")))
}

/// The fake's server: this version, this tool set, this window, and NO
/// digest - a server that says nothing about its list.
fn server(version: &str) -> impl Fn(&str, &Value) -> Reply + Send + 'static {
    let version = version.to_string();
    move |method, _| match method {
        "server/discover" => Reply::Result(super::fake::discover_as(
            &[PROTOCOL_VERSION],
            "mcp_host",
            &version,
        )),
        "tools/list" => Reply::Result(tools_cached(&TOOLS, TTL_MS)),
        _ => Reply::Error(-32601, "no"),
    }
}

/// The fake's server: this version, this tool set, this window, and this
/// catalogue digest.
fn server_with_digest(
    version: &str,
    digest: &str,
) -> impl Fn(&str, &Value) -> Reply + Send + 'static {
    let version = version.to_string();
    let digest = digest.to_string();
    move |method, _| match method {
        "server/discover" => Reply::Result(discover_digest_as(
            &[PROTOCOL_VERSION],
            "mcp_host",
            &version,
            &digest,
        )),
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

/// **A CHANGED DIGEST PUTS THE KEPT LIST ASIDE.** The server published the
/// digest of the list it serves, and the digest is not the one the kept
/// list was taken under: the tool table moved, and the host is asked again.
///
/// The version and the absent set are the same as they were - a table can
/// move without either of them moving - so this is the case the OLD key
/// could not see. The digest is the server's own word, and it is what
/// catches this.
#[tokio::test]
async fn a_changed_digest_puts_the_kept_list_aside() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(&dir);
    let (r, w, _) = serve(server_with_digest("1.99.19", DIGEST));
    let _ = Client::connect_with(r, w, T, &store).await.unwrap();

    // The same binary, the same tool set, and a DIFFERENT digest: the
    // table moved without a version bump.
    let (r, w, seen) = serve(|method, _| match method {
        "server/discover" => Reply::Result(discover_digest_as(
            &[PROTOCOL_VERSION],
            "mcp_host",
            "1.99.19",
            OTHER_DIGEST,
        )),
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
        "a changed digest did not put the kept list aside"
    );
    assert!(
        after.has_tool("qbit.torrents"),
        "the tool the table gained is not in the panel"
    );
    match after.catalogue {
        Catalogue::Relisted {
            miss: Miss::Changed { kept, now },
            ..
        } => {
            assert_eq!(kept, DIGEST, "the digest the kept list was taken under");
            assert_eq!(now, OTHER_DIGEST, "the digest the server publishes now");
        }
        other => panic!("a changed digest is not named: {other:?}"),
    }
}

/// **AN UNCHANGED DIGEST KEEPS THE KEPT LIST.** The server's own word is
/// the same one the kept list was taken under, so the kept copy IS the
/// list the server serves: the host is not asked for the list again, and
/// the answer says the digest matched.
#[tokio::test]
async fn an_unchanged_digest_keeps_the_kept_list() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(&dir);
    let (r, w, _) = serve(server_with_digest("1.99.19", DIGEST));
    let _ = Client::connect_with(r, w, T, &store).await.unwrap();

    let (r, w, seen) = serve(server_with_digest("1.99.19", DIGEST));
    let second = Client::connect_with(r, w, T, &store).await.unwrap();
    assert_eq!(
        methods(&seen),
        ["server/discover"],
        "the server was asked for a list that had not changed"
    );
    match second.catalogue {
        Catalogue::Kept { proof, .. } => assert_eq!(
            proof,
            Proof::Digest {
                digest: DIGEST.to_string()
            },
            "the answer does not say the server's digest matched"
        ),
        other => panic!("the kept list was not used: {other:?}"),
    }
}

/// **A RELEASE THAT CHANGED NO TOOL KEEPS THE KEPT LIST.** The version
/// moved - a release - but the digest did not: the release touched no tool
/// table, so the list the client holds IS the list the server serves. The
/// host is not asked for the list again, and the whole catalogue is not
/// pulled off the host for nothing.
#[tokio::test]
async fn a_release_that_changed_no_tool_keeps_the_kept_list() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(&dir);
    let (r, w, _) = serve(server_with_digest("1.99.19", DIGEST));
    let first = Client::connect_with(r, w, T, &store).await.unwrap();

    let (r, w, seen) = serve(server_with_digest("1.99.20", DIGEST));
    let second = Client::connect_with(r, w, T, &store).await.unwrap();
    assert_eq!(
        methods(&seen),
        ["server/discover"],
        "a release re-listed a catalogue that had not changed"
    );
    assert_eq!(
        second.tools, first.tools,
        "the kept list changed under a release that touched no tool"
    );
    assert!(
        matches!(second.catalogue, Catalogue::Kept { .. }),
        "the release put a list that had not changed aside: {:?}",
        second.catalogue
    );
}

/// **THE DIGEST THE SERVER HANDED IS KEPT.** The digest is of the WHOLE
/// list - names AND schemas - and this client keeps only the names, so it
/// cannot compute the digest again: it keeps the server's word, beside the
/// list the word was given for.
#[tokio::test]
async fn the_digest_the_server_handed_is_kept() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(&dir);
    let (r, w, _) = serve(server_with_digest("1.99.19", DIGEST));
    let _ = Client::connect_with(r, w, T, &store).await.unwrap();
    let text = std::fs::read_to_string(dir.path().join("catalogue.json")).unwrap();
    assert!(
        text.contains(DIGEST),
        "the digest was not kept with the list: {text}"
    );
}

/// **A SERVER THAT PUBLISHES NO DIGEST DOES NOT RE-LIST EVERY SESSION.**
/// The fact is absent - the server said nothing about its list - but the
/// client still keeps the list it has, the way it always did. A client
/// that re-listed every session would pay the whole catalogue for every
/// gather, which is the defect the digest closes and not a new one.
///
/// And the answer says the fact was ABSENT: a check that did not happen is
/// not an all-clear (servers ROADMAP E72).
#[tokio::test]
async fn a_server_that_publishes_no_digest_still_keeps_the_list() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(&dir);
    let (r, w, _) = serve(server("1.99.19"));
    let _ = Client::connect_with(r, w, T, &store).await.unwrap();
    let (r, w, seen) = serve(server("1.99.19"));
    let second = Client::connect_with(r, w, T, &store).await.unwrap();
    assert_eq!(
        methods(&seen),
        ["server/discover"],
        "a server that publishes no digest was asked for the list again"
    );
    assert_eq!(second.tools, TOOLS);
    match second.catalogue {
        Catalogue::Kept { proof, .. } => assert_eq!(
            proof,
            Proof::Absent,
            "the answer does not say the digest was absent"
        ),
        other => panic!("the kept list was not used: {other:?}"),
    }
}

/// **A KEPT LIST TAKEN WITHOUT A DIGEST IS PUT ASIDE, ONCE.** The client
/// cannot check a list it holds no digest for, and a list it cannot check
/// is not a list it may use - so it is put aside and the server is asked
/// again. The digest is then remembered beside the list, and the next
/// session is quiet.
#[tokio::test]
async fn a_kept_list_taken_without_a_digest_is_put_aside_once() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(&dir);
    // A list kept by an older client: no digest beside it.
    store
        .store("1.99.19", "", None, &TOOLS.map(str::to_string), TTL_MS)
        .expect("the list is kept");
    let (r, w, seen) = serve(server_with_digest("1.99.19", DIGEST));
    let after = Client::connect_with(r, w, T, &store).await.unwrap();
    assert_eq!(
        methods(&seen),
        ["server/discover", "tools/list"],
        "a list that could not be checked was used"
    );
    match after.catalogue {
        Catalogue::Relisted {
            miss: Miss::Uncheckable,
            ..
        } => {}
        other => panic!("an uncheckable list is not named: {other:?}"),
    }
    // And the digest is remembered now, so the next session is quiet.
    let (r, w, seen2) = serve(server_with_digest("1.99.19", DIGEST));
    let second = Client::connect_with(r, w, T, &store).await.unwrap();
    assert_eq!(
        methods(&seen2),
        ["server/discover"],
        "the digest was not remembered beside the list"
    );
    assert!(matches!(second.catalogue, Catalogue::Kept { .. }));
}

/// **A DIGEST THAT IS NOT A DIGEST IS NO DIGEST.** A server that publishes
/// something this client cannot check has said nothing about its list: the
/// kept copy is checked the older way and the answer says the fact was
/// absent, rather than the client comparing strings it does not understand.
#[tokio::test]
async fn a_digest_that_is_not_a_digest_is_no_digest() {
    let dir = tempfile::tempdir().unwrap();
    let store = store(&dir);
    let (r, w, _) = serve(server_with_digest("1.99.19", DIGEST));
    let _ = Client::connect_with(r, w, T, &store).await.unwrap();

    let (r, w, seen) = serve(server_with_digest("1.99.19", "not-a-digest"));
    let second = Client::connect_with(r, w, T, &store).await.unwrap();
    assert_eq!(
        methods(&seen),
        ["server/discover"],
        "a digest the client cannot check put the kept list aside"
    );
    match second.catalogue {
        Catalogue::Kept { proof, .. } => assert_eq!(
            proof,
            Proof::Absent,
            "a digest the client cannot check was drawn as a check"
        ),
        other => panic!("the kept list was not used: {other:?}"),
    }
}
