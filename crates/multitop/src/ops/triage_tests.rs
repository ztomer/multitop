//! `machine.triage` in the Ops view (servers ROADMAP O92): the machine's one
//! verdict, at the top of the card, in the server's own words.

use serde_json::{json, Value};

use super::render::render;
use super::tests::{pal, plain, snapshot};
use super::*;

const NOW: i64 = 1_790_170_000;

/// What `.90` answered on 2026-10-24 (`media_server` 1.99.25): two stacks,
/// nothing to say.
fn storage_quiet() -> Value {
    json!({"machine":"storage","needs_attention":false,"items":[],
        "quiet":["instance storage-server: 8 suite checks ok (oldest 3 min ago); 1 other scheduled check ok (oldest 17 h ago)",
                 "instance solver: 7 suite checks ok (oldest 3 min ago); 1 other scheduled check ok (oldest 17 h ago)"]})
}

/// The design's example: the SUSPENDED pool, with ZFS's own fix, and a
/// failing check whose owner states no remedy.
fn storage_in_trouble() -> Value {
    json!({"machine":"storage","needs_attention":true,
        "items":[
            {"instance":"storage-server","kind":"zfs_pool",
             "what":"pool0 is SUSPENDED: member /dev/sdb is REMOVED",
             "witnesses":["zfs.health: pool SUSPENDED, action: attach the missing device"],
             "fix":"Make sure the affected devices are connected, then run zpool clear."},
            {"instance":"solver","kind":"check","what":"check_vpn_leak FAIL (rc 1)",
             "witnesses":["health: native:check_vpn_leak FAIL (rc 1)"],"fix":null}],
        "quiet":[]})
}

fn frame(triage: Part<Triage>) -> Vec<String> {
    let mut s = snapshot();
    s.triage = triage;
    plain(&render(
        "storage",
        &OpsState::Ready(Box::new(s)),
        100,
        60,
        NOW,
        &pal(),
    ))
}

#[test]
fn a_quiet_machine_says_so_and_says_what_was_read_on_each_stack() {
    let f = frame(Part::Ready(parse_triage(&storage_quiet()).unwrap()));
    // At the top of the card: right under the server's own lines.
    assert_eq!(
        &f[3..8],
        [
            "✓ triage     nothing needs your hand",
            "· triage     instance storage-server: 8 suite checks ok (oldest 3 min ago)",
            "    1 other scheduled check ok (oldest 17 h ago)",
            "· triage     instance solver: 7 suite checks ok (oldest 3 min ago)",
            "    1 other scheduled check ok (oldest 17 h ago)",
        ]
    );
}

#[test]
fn each_item_names_its_stack_and_quotes_the_owners_fix_or_says_there_is_none() {
    let f = frame(Part::Ready(parse_triage(&storage_in_trouble()).unwrap()));
    assert_eq!(
        &f[3..9],
        [
            "✗ triage     storage-server: pool0 is SUSPENDED: member /dev/sdb is REMOVED",
            "    fix: Make sure the affected devices are connected, then run zpool clear.",
            "    zfs.health: pool SUSPENDED, action: attach the missing device",
            "✗ triage     solver: check_vpn_leak FAIL (rc 1)",
            "    fix: none stated by the owner",
            "    health: native:check_vpn_leak FAIL (rc 1)",
        ]
    );
}

#[test]
fn attention_with_nothing_named_is_never_drawn_as_all_clear() {
    let t = parse_triage(&json!({"needs_attention": true, "items": [], "quiet": []})).unwrap();
    let f = frame(Part::Ready(t));
    assert_eq!(
        f[3],
        "⚠ triage     the server says something needs your hand, and names nothing"
    );
}

#[test]
fn an_item_without_its_stack_is_refused_not_drawn() {
    let mut v = storage_in_trouble();
    v["items"][0].as_object_mut().unwrap().remove("instance");
    assert_eq!(
        parse_triage(&v),
        Err("machine.triage: an item has no instance".into())
    );
    assert!(parse_triage(&json!({"items": [], "quiet": []})).is_err());
    let mut bad_fix = storage_in_trouble();
    bad_fix["items"][0]["fix"] = json!(7);
    assert!(parse_triage(&bad_fix).is_err());
}

#[test]
fn a_gather_asks_the_machine_for_its_verdict() {
    let raw = cache::RawSnapshot {
        triage: cache::Raw::Value(storage_quiet()),
        ..super::cache_tests::raw(NOW)
    };
    let s = parse_snapshot(&raw);
    assert!(
        matches!(s.triage, Part::Ready(ref t) if t.quiet.len() == 2),
        "{:?}",
        s.triage
    );
    let raw = cache::RawSnapshot {
        triage: cache::Raw::Value(storage_in_trouble()),
        ..super::cache_tests::raw(NOW)
    };
    // `/solver` finds the machine whose solver stack is in trouble.
    assert!(parse_snapshot(&raw).terms().contains(&"solver"));
}
