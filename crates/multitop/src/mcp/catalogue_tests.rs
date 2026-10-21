//! The catalogue's own rules, proved where they can be got wrong.
//!
//! Five things, and each is a way the kept list could lie:
//!
//! - the KEY: the server's own word - the DIGEST of the list it serves -
//!   when the server publishes one, and the version and the absent set
//!   when it does not;
//! - the WINDOW: the server's own `ttlMs` bounds the entry, and a window of
//!   zero is a live fact with nothing to keep;
//! - the SET: the absent set is a set, so the order the server published it
//!   in is not part of what it says;
//! - the WHOLE LIST: what is kept is every tool the server listed, so a
//!   tool the cache omits cannot vanish from the panel;
//! - the ABSENCE: a digest the client cannot check says the fact is
//!   absent, and never means "the copy is fine".

use std::collections::HashMap;

use super::catalogue::{fingerprint, is_digest, Entry, Hit, Miss, Proof, Store};

/// The window the estate's servers give a list: an hour, in milliseconds.
const TTL_MS: u64 = 3_600_000;
/// A digest in the estate's spelling.
const DIGEST: &str = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
/// Another digest, for a catalogue that moved.
const OTHER: &str = "sha256:fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210";

/// An entry, as the store keeps one.
fn entry(version: &str, absent: &str, ttl_ms: u64, fetched_at: i64) -> Entry {
    Entry {
        version: version.to_string(),
        absent: absent.to_string(),
        digest: None,
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
        .store("1.99.19", "", None, &tools, TTL_MS)
        .expect("the list is kept");
    let Hit::Current { tools: kept, .. } = store.hit("1.99.19", "", None) else {
        panic!(
            "the kept list was not used: {:?}",
            store.hit("1.99.19", "", None)
        );
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
    assert!(matches!(
        store.hit("1.99.19", "", None),
        Hit::Missed(Miss::None)
    ));
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
            None,
            &["cron.status".to_string()],
            TTL_MS,
        )
        .expect("the list is kept");
    match store.hit("1.99.20", "docker.containers", None) {
        Hit::Missed(Miss::Released { kept, now }) => {
            assert_eq!(kept, "1.99.19");
            assert_eq!(now, "1.99.20");
        }
        other => panic!("a release must put the kept list aside: {other:?}"),
    }
    match store.hit("1.99.19", "docker.containers,zfs.health", None) {
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
        .store("1.99.19", "", None, &["cron.status".to_string()], TTL_MS)
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
    match store.hit("1.99.19", "", None) {
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
        .store("", "", None, &["cron.status".to_string()], TTL_MS)
        .expect("the write is refused, not the process");
    assert!(matches!(store.hit("", "", None), Hit::Missed(Miss::None)));
}

/// **THE DIGEST IS THE KEY, WHEN THE SERVER PUBLISHES ONE.** The digest
/// is the server's own word about the list it serves: a digest that
/// MATCHES keeps the list, even when the version moved - a release that
/// touched no tool table is not a change to the list - and a digest that
/// does not match puts the list aside.
#[test]
fn the_digest_is_the_key_when_the_server_publishes_one() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::at(Some(dir.path().join("catalogue.json")));
    store
        .store(
            "1.99.19",
            "",
            Some(DIGEST),
            &["cron.status".to_string()],
            TTL_MS,
        )
        .expect("the list is kept");
    // The same digest, and a NEW version: the release touched no tool.
    let Hit::Current { proof, .. } = store.hit("1.99.20", "", Some(DIGEST)) else {
        panic!(
            "the kept list was not used: {:?}",
            store.hit("1.99.20", "", Some(DIGEST))
        );
    };
    assert_eq!(
        proof,
        Proof::Digest {
            digest: DIGEST.to_string()
        }
    );
    // Another digest: the table moved.
    match store.hit("1.99.20", "", Some(OTHER)) {
        Hit::Missed(Miss::Changed { kept, now }) => {
            assert_eq!(kept, DIGEST);
            assert_eq!(now, OTHER);
        }
        other => panic!("a changed digest must put the kept list aside: {other:?}"),
    }
}

/// **A KEPT LIST TAKEN WITHOUT A DIGEST IS NOT USED AGAINST ONE.** The
/// client cannot check a list it has no digest for, and a list it cannot
/// check is not a list it may use: it is put aside, and the server is
/// asked again - once, and then the digest is remembered with the list.
#[test]
fn a_kept_list_taken_without_a_digest_is_not_used_against_one() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::at(Some(dir.path().join("catalogue.json")));
    store
        .store("1.99.19", "", None, &["cron.status".to_string()], TTL_MS)
        .expect("the list is kept");
    assert!(matches!(
        store.hit("1.99.19", "", Some(DIGEST)),
        Hit::Missed(Miss::Uncheckable)
    ));
}

/// **A DIGEST THE CLIENT CANNOT CHECK SAYS THE FACT IS ABSENT.** A server
/// that publishes no digest has said nothing about its list: the kept list
/// is checked the older way, and the answer says the digest was absent -
/// which is NOT the same as saying the copy is fine.
#[test]
fn a_digest_the_client_cannot_check_says_the_fact_is_absent() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::at(Some(dir.path().join("catalogue.json")));
    store
        .store(
            "1.99.19",
            "",
            Some(DIGEST),
            &["cron.status".to_string()],
            TTL_MS,
        )
        .expect("the list is kept");
    let Hit::Current { proof, .. } = store.hit("1.99.19", "", None) else {
        panic!(
            "the kept list was not used: {:?}",
            store.hit("1.99.19", "", None)
        );
    };
    assert_eq!(proof, Proof::Absent);
}

/// **ONLY THE ESTATE'S SPELLING IS A DIGEST.** A digest without its
/// algorithm is a number nobody can check, and one in the wrong shape is
/// not a digest at all: the client says the fact is absent rather than
/// comparing strings it does not understand.
#[test]
fn only_the_estates_spelling_is_a_digest() {
    assert!(is_digest(DIGEST));
    assert!(!is_digest("0123456789abcdef"), "no algorithm, no digest");
    assert!(!is_digest("sha256:"), "no hex, no digest");
    assert!(!is_digest("md5:0123456789abcdef"), "another algorithm, no");
    assert!(!is_digest(&format!("sha256:{}", "z".repeat(64))));
    assert!(
        !is_digest(&format!("sha256:{}", "A".repeat(64))),
        "not the estate's hex"
    );
    assert!(
        !is_digest(&format!("sha256:{}", "a".repeat(63))),
        "too short"
    );
    assert!(
        !is_digest(&format!("sha256:{}", "a".repeat(65))),
        "too long"
    );
}
