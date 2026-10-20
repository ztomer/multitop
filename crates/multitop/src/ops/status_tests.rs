//! The STATUSES: an enum, a named vocabulary on the wire, and a glyph the
//! RENDERER picks (DECISIONS D14, owner 2026-10-20).
//!
//! *"V is a visual representation of a status. we should use enums, not utf
//! symbols. it shouldn't be something we transport around."*
//!
//! Three claims, checked here:
//!
//! - the wire carries a NAME, and the name is the estate's;
//! - a name this client does not have is REFUSED (ours) or kept VERBATIM
//!   (another owner's), never guessed at;
//! - the frame draws the SAME glyphs it drew before - the same picture,
//!   chosen by the renderer.

use serde_json::json;

use super::render::render;
use super::status::{DockerHealth, DockerState, Verdict};
use super::tests::{pal, plain, snapshot};
use super::*;

/// The frame's clock: the same instant every Ops test renders at.
const NOW: i64 = 1_790_170_000;

/// A verdict this client does not have is REFUSED: the section is unreadable
/// rather than read as some verdict it is not. `unknown` means "nothing was
/// measured", not "I do not know this word". The enum grows - the string
/// does not.
#[test]
fn a_verdict_we_do_not_have_is_refused() {
    let answer = json!({"tunnels": [
        {"instance": "solver", "container": "solver-vpn", "leak": "leaked-ish"},
    ]});
    let err = parse_egress(&answer).expect_err("an unknown verdict must not read");
    assert!(
        err.contains("leaked-ish"),
        "the reason names the word: {err}"
    );
}

/// A word ANOTHER OWNER may add is kept VERBATIM: the frame shows what the
/// server published, and a paraphrase would show something else.
#[test]
fn a_foreign_word_is_kept_verbatim() {
    let answer = json!({"pools": [{"name": "pool0", "state": "REPLACED", "members": [
        {"name": "sdb", "state": "REPLACED", "read": 0, "write": 0, "cksum": 0}
    ]}]});
    let pools = parse_zfs(&answer).unwrap();
    assert_eq!(pools[0].state.as_str(), "REPLACED");
    assert_eq!(pools[0].members[0].state.as_str(), "REPLACED");
    // And the estate's VERDICT about the pool is a different fact, and it
    // is ours: a pool that is not ONLINE is not whole.
    assert_eq!(pools[0].verdict(), Verdict::NotWhole);
}

/// The estate's verdict about a pool is OURS, and it is a different fact
/// from ZFS's word: ZFS says ONLINE; the estate says NOT WHOLE, because a
/// member is gone.
#[test]
fn the_pools_verdict_is_ours_and_the_zfs_word_is_theirs() {
    let answer = json!({"pools": [{"name": "pool0", "state": "ONLINE", "members": [
        {"name": "mirror-0", "state": "ONLINE", "read": 0, "write": 0, "cksum": 0},
        {"name": "sdb", "state": "REMOVED", "read": 0, "write": 0, "cksum": 0}
    ]}]});
    let pools = parse_zfs(&answer).unwrap();
    assert_eq!(pools[0].state.as_str(), "ONLINE");
    assert_eq!(pools[0].verdict(), Verdict::NotWhole);
    assert!(!pools[0].is_whole());
}

/// THE SAME FRAME, THE SAME GLYPHS. A machine whose tunnel leaked and whose
/// pool lost a member: the frame reads exactly as it read before the
/// statuses became enums, because the picture is the renderer's and the
/// renderer did not change.
#[test]
fn the_frame_draws_the_same_glyphs_it_drew_before() {
    let mut s = snapshot();
    s.vpn = Part::Ready(
        parse_egress(&json!({
            "egress": "BR (POSSIBLE LEAK)",
            "tunnels": [{"instance": "solver", "container": "solver-vpn", "leak": "leaked"}],
        }))
        .unwrap(),
    );
    s.zfs = Part::Ready(
        parse_zfs(&json!({"pools": [{"name": "pool0", "state": "SUSPENDED", "action": "attach the missing device",
            "msg": "", "scan": "", "members": [
                {"name": "sdb", "state": "REMOVED", "read": 0, "write": 0, "cksum": 0}
            ]}]}))
        .unwrap(),
    );
    let f = plain(&render(
        "storage",
        &OpsState::Ready(Box::new(s)),
        80,
        40,
        NOW,
        &pal(),
    ));
    let has = |want: &str| f.iter().any(|l| l.contains(want));
    assert!(
        has("✗ vpn        1 tunnel, 1 leaked"),
        "the leaked tunnel reads as it always did: {f:?}"
    );
    assert!(
        has("solver-vpn leaked via solver"),
        "the tunnel's own line is the server's words: {f:?}"
    );
    assert!(
        has("✗ zfs        1 of 1 not whole"),
        "the broken pool reads as it always did: {f:?}"
    );
    assert!(
        has("pool0  SUSPENDED · sdb REMOVED"),
        "the member that is gone is named, in ZFS's own words: {f:?}"
    );
}

/// A container's state and health are DOCKER's words, parsed at the edge.
#[test]
fn the_container_words_are_dockers_own() {
    let answer = json!({"containers": [
        {"name": "a", "state": "running", "status": "Up 1 hour (healthy)", "health": "healthy"},
        {"name": "b", "state": "exited", "status": "Exited (0)", "health": "none"},
    ]});
    let cs = parse_containers(&answer).unwrap();
    assert_eq!(cs[0].state, DockerState::Running);
    assert_eq!(cs[0].health, DockerHealth::Healthy);
    assert_eq!(cs[1].state, DockerState::Exited);
    assert_eq!(cs[1].health, DockerHealth::NoCheck);
    assert!(!cs[0].health.is_unchecked());
    assert!(cs[1].health.is_unchecked());
}
