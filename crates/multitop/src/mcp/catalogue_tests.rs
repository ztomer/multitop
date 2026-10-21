//! The catalogue's own rules, proved where they can be got wrong.
//!
//! Four things, and each is a way the kept list could lie:
//!
//! - the KEY: the list is kept under the server's advertised version and the
//!   set of tools it says it does not offer, and under nothing else;
//! - the WINDOW: the server's own `ttlMs` bounds the entry, and a window of
//!   zero is a live fact with nothing to keep;
//! - the SET: the absent set is a set, so the order the server published it
//!   in is not part of what it says;
//! - the WHOLE LIST: what is kept is every tool the server listed, so a
//!   tool the cache omits cannot vanish from the panel.

use std::collections::HashMap;

use super::catalogue::{fingerprint, Entry, Hit, Miss, Store};

/// The window the estate's servers give a list: an hour, in milliseconds.
const TTL_MS: u64 = 3_600_000;

/// An entry, as the store keeps one.
fn entry(version: &str, absent: &str, ttl_ms: u64, fetched_at: i64) -> Entry {
    Entry {
        version: version.to_string(),
        absent: absent.to_string(),
        tools: vec!["cron.status".to_string(), "health.list".to_string()],
        ttl_ms,
        fetched_at,
    }
}

/// **THE KEY IS THE VERSION AND THE ABSENT SET.** A release bumps the
/// version, and a host that gains or loses a tool changes the absent set:
/// both put the kept list aside.
#[test]
fn the_key_is_the_version_and_the_absent_set() {
    let e = entry("1.99.19", "docker.containers", TTL_MS, 1_000);
    assert!(e.is_for("1.99.19", "docker.containers"));
    assert!(
        !e.is_for("1.99.20", "docker.containers"),
        "a release moves the version, and the kept list is for the old binary"
    );
    assert!(
        !e.is_for("1.99.19", "docker.containers,zfs.health"),
        "a host that gained a tool changes the absent set"
    );
}

/// **THE SERVER'S OWN WINDOW BOUNDS THE ENTRY.** `ttlMs` is the answer's
/// owner saying how long the answer may be kept, and a kept list that
/// outlived it would be caching something the server never promised.
#[test]
fn the_servers_own_window_bounds_the_entry() {
    let e = entry("1.99.19", "", TTL_MS, 1_000);
    assert!(e.is_fresh(1_000));
    assert!(e.is_fresh(1_000 + 3_599));
    assert!(
        !e.is_fresh(1_000 + 3_600),
        "the window closed an hour later"
    );
    // A window of zero is the protocol's "a live fact": there is nothing to
    // keep, so there is nothing to use.
    let e = entry("1.99.19", "", 0, 1_000);
    assert!(!e.is_fresh(1_000));
}

/// **THE ABSENT SET IS A SET, NOT A LIST.** The order the server published
/// it in is not part of what it says, so the key must not change when the
/// order does.
#[test]
fn the_absent_set_is_a_set_not_a_list() {
    let mut a = HashMap::new();
    a.insert("zfs.health".to_string(), "no ZFS here".to_string());
    a.insert(
        "docker.containers".to_string(),
        "no Docker here".to_string(),
    );
    let mut b = HashMap::new();
    b.insert(
        "docker.containers".to_string(),
        "no Docker here".to_string(),
    );
    b.insert("zfs.health".to_string(), "no ZFS here".to_string());
    assert_eq!(fingerprint(&a), fingerprint(&b));
    assert_eq!(fingerprint(&a), "docker.containers,zfs.health");
    assert_eq!(fingerprint(&HashMap::new()), "");
}

/// **THE REASONS ARE NOT PART OF THE KEY.** The reason a tool is absent is
/// the server's own words, read live on every session; only the SET of
/// names decides which list the server is offering.
#[test]
fn the_reasons_are_not_part_of_the_key() {
    let mut a = HashMap::new();
    a.insert("zfs.health".to_string(), "no ZFS here".to_string());
    let mut b = HashMap::new();
    b.insert(
        "zfs.health".to_string(),
        "the pool device is missing".to_string(),
    );
    assert_eq!(fingerprint(&a), fingerprint(&b));
}

/// **THE KEPT LIST IS THE WHOLE LIST.** The store keeps every name it was
/// given: a list that lost a tool would lose that tool's section from the
/// panel, and the cache is an optimisation, never a filter.
#[test]
fn the_kept_list_is_the_whole_list() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::at(Some(dir.path().join("catalogue.json")));
    let tools: Vec<String> = (0..40).map(|i| format!("tool.{i}")).collect();
    store
        .store("1.99.19", "", &tools, TTL_MS)
        .expect("the list is kept");
    let Hit::Current { tools: kept, .. } = store.hit("1.99.19", "") else {
        panic!("the kept list was not used: {:?}", store.hit("1.99.19", ""));
    };
    assert_eq!(kept, tools, "the kept list lost a tool");
}

/// **A FILE THAT CANNOT BE READ IS NO LIST.** It is not a wrong list and it
/// is not a crash: the list is gone, and the server is asked again.
#[test]
fn a_file_that_cannot_be_read_is_no_list() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("catalogue.json");
    std::fs::write(&path, "{not json at all").unwrap();
    let store = Store::at(Some(path));
    assert!(matches!(store.hit("1.99.19", ""), Hit::Missed(Miss::None)));
}

/// **A LIST FOR ANOTHER SERVER IS NOT USED, AND THE CLIENT IS TOLD WHY.**
#[test]
fn a_list_for_another_server_is_not_used() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::at(Some(dir.path().join("catalogue.json")));
    store
        .store(
            "1.99.19",
            "docker.containers",
            &["cron.status".to_string()],
            TTL_MS,
        )
        .expect("the list is kept");
    match store.hit("1.99.20", "docker.containers") {
        Hit::Missed(Miss::Released { kept, now }) => {
            assert_eq!(kept, "1.99.19");
            assert_eq!(now, "1.99.20");
        }
        other => panic!("a release must put the kept list aside: {other:?}"),
    }
    match store.hit("1.99.19", "docker.containers,zfs.health") {
        Hit::Missed(Miss::Reshaped { kept, now }) => {
            assert_eq!(kept, "docker.containers");
            assert_eq!(now, "docker.containers,zfs.health");
        }
        other => panic!("a changed tool set must put the kept list aside: {other:?}"),
    }
}

/// **A LIST PAST ITS WINDOW IS NOT USED, AND THE CLIENT IS TOLD WHY.**
#[test]
fn a_list_past_its_window_is_not_used() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::at(Some(dir.path().join("catalogue.json")));
    store
        .store("1.99.19", "", &["cron.status".to_string()], TTL_MS)
        .expect("the list is kept");
    // The entry was taken an hour and a half ago, and the server's window is
    // an hour.
    let entry: Entry =
        serde_json::from_str(&std::fs::read_to_string(dir.path().join("catalogue.json")).unwrap())
            .unwrap();
    let old = Entry {
        fetched_at: entry.fetched_at - 5_400,
        ..entry
    };
    std::fs::write(
        dir.path().join("catalogue.json"),
        serde_json::to_string(&old).unwrap(),
    )
    .unwrap();
    match store.hit("1.99.19", "") {
        Hit::Missed(Miss::Expired { age_s, ttl_s }) => {
            assert!(age_s >= 5_400, "the age is the truth: {age_s}");
            assert_eq!(ttl_s, 3_600);
        }
        other => panic!("a list past its window must not be used: {other:?}"),
    }
}

/// **A SERVER THAT NAMES NO VERSION IS NOT KEYED, SO NOTHING IS KEPT.**
/// The version is the only thing that says which binary holds the tool
/// table; without it a kept list could outlive the tool it names.
#[test]
fn a_server_that_names_no_version_keeps_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::at(Some(dir.path().join("catalogue.json")));
    store
        .store("", "", &["cron.status".to_string()], TTL_MS)
        .expect("the write is refused, not the process");
    assert!(matches!(store.hit("", ""), Hit::Missed(Miss::None)));
}
