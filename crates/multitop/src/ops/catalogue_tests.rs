//! The catalogue, as the Ops view draws it.
//!
//! servers ROADMAP E61: the tool list is the panel's SHAPE - a tool the
//! list does not name has no section - so where the list came from, and how
//! old it is, is part of what the panel says. A panel that cannot say it
//! could be showing yesterday's tools as today's, and a kept list that had
//! to be put aside has to be said to be stale rather than quietly replaced.

use super::render::render;
use super::tests::{pal, plain};
use super::{Catalogue, Miss, OpsState, Part, Proof, Snapshot};

const NOW: i64 = 1_790_170_000;

/// A snapshot of a host whose tool list came from the server.
fn snapshot() -> Snapshot {
    Snapshot {
        server: ("mcp_host".into(), "1.99.19".into()),
        at: NOW - 12,
        sampled_every_s: 450,
        catalogue: Catalogue::Listed { ttl_s: 3_600 },
        health: Part::Ready(super::model::Health {
            checks: vec![],
            oldest_age: None,
            oldest_age_s: None,
            cadence: super::model_health::Cadence {
                secs: Some(900),
                words: Some("every 15 min".into()),
            },
        }),
        vpn: Part::Absent("this host lists no net.egress".into()),
        cron: Part::Ready(Vec::new()),
        containers: Part::Absent("no Docker on this host".into()),
        zfs: Part::Absent("this host lists no zfs.health".into()),
        disk: Part::Absent("this host lists no disk.usage".into()),
        facts: Part::Absent("this host lists no host.facts".into()),
        alerts: Part::Ready(Vec::new()),
    }
}

/// The frame for `s`, as plain lines.
fn frame(s: Snapshot) -> Vec<String> {
    plain(&render(
        "beelink",
        &OpsState::Ready(Box::new(s)),
        80,
        40,
        NOW,
        &pal(),
    ))
}

/// **THE PANEL SAYS WHERE ITS SHAPE CAME FROM.** The tool list is the
/// panel's shape: a tool the list does not name has no section. So a panel
/// that cannot say where its shape came from could be showing yesterday's
/// tools as today's, and the kept list's age is on the line.
#[test]
fn the_frame_says_where_the_tool_list_came_from() {
    let mut s = snapshot();
    s.catalogue = Catalogue::Kept {
        age_s: 12 * 60,
        ttl_s: 3_600,
        proof: Proof::Digest {
            digest: "sha256:0123".to_string(),
        },
    };
    let f = frame(s);
    assert!(
        f.iter()
            .any(|l| l.contains("tool list: kept 12m old (window 1h)")),
        "the frame does not say how old the kept list is: {f:?}"
    );
}

/// **A LIST THAT WAS PUT ASIDE IS SAID TO BE STALE.** A kept list past the
/// server's own window is not used, and the frame says the kept one had
/// expired - a panel that quietly drew a fresh list over an old one would
/// hide the very thing the operator needs to know.
#[test]
fn the_frame_says_a_stale_list_was_stale() {
    let mut s = snapshot();
    s.catalogue = Catalogue::Relisted {
        miss: Miss::Expired {
            age_s: 7_200,
            ttl_s: 3_600,
        },
        ttl_s: 3_600,
    };
    let f = frame(s);
    assert!(
        f.iter()
            .any(|l| l
                .contains("tool list: re-listed, the kept list had expired (2h old, window 1h)")),
        "the frame does not say the kept list had expired: {f:?}"
    );
}

/// **A RELEASE IS SAID TOO.** The kept list was for another binary, so it
/// was not used - and the frame says which binary it was for.
#[test]
fn the_frame_says_a_release_put_the_kept_list_aside() {
    let mut s = snapshot();
    s.catalogue = Catalogue::Relisted {
        miss: Miss::Released {
            kept: "1.99.18".into(),
            now: "1.99.19".into(),
        },
        ttl_s: 3_600,
    };
    let f = frame(s);
    assert!(
        f.iter()
            .any(|l| l.contains("tool list: re-listed, the kept list was for 1.99.18")),
        "the frame does not say the kept list was for another binary: {f:?}"
    );
}

/// **THE LIST THE SERVER JUST GAVE IS SAID AS SUCH.** No age, because
/// there is none: the list came from the server, now.
#[test]
fn the_frame_says_the_list_came_from_the_server() {
    let f = frame(snapshot());
    assert!(
        f.iter()
            .any(|l| l.contains("tool list: from the server (window 1h)")),
        "the frame does not say where the list came from: {f:?}"
    );
}

/// **THE PANEL SAYS THE DIGEST WAS CHECKED.** A kept list is the server's
/// list only if the server says so, and the digest is how it says so: the
/// panel has to say the check happened, or a reader cannot tell a checked
/// copy from an unchecked one.
#[test]
fn the_frame_says_the_digest_was_checked() {
    let mut s = snapshot();
    s.catalogue = Catalogue::Kept {
        age_s: 12 * 60,
        ttl_s: 3_600,
        proof: Proof::Digest {
            digest: "sha256:0123".to_string(),
        },
    };
    let f = frame(s);
    assert!(
        f.iter()
            .any(|l| l.contains("tool list: kept 12m old (window 1h), the server's digest matches")),
        "the frame does not say the server's digest matched: {f:?}"
    );
}

/// **A DIGEST THAT WAS ABSENT IS SAID TO BE ABSENT.** The server published
/// none, so the kept copy was checked the older way - and the panel has to
/// say that, because a check that did not happen is not an all-clear.
#[test]
fn the_frame_says_the_digest_was_absent() {
    let mut s = snapshot();
    s.catalogue = Catalogue::Kept {
        age_s: 12 * 60,
        ttl_s: 3_600,
        proof: Proof::Absent,
    };
    let f = frame(s);
    assert!(
        f.iter()
            .any(|l| l.contains("the server published no digest")),
        "the frame does not say the digest was absent: {f:?}"
    );
}

/// **A CHANGED DIGEST IS SAID TOO.** The kept list was for another
/// catalogue, so it was not used - and the frame says the server's own
/// word moved.
#[test]
fn the_frame_says_the_digest_changed() {
    let mut s = snapshot();
    s.catalogue = Catalogue::Relisted {
        miss: Miss::Changed {
            kept: "sha256:0123".into(),
            now: "sha256:4567".into(),
        },
        ttl_s: 3_600,
    };
    let f = frame(s);
    assert!(
        f.iter()
            .any(|l| l.contains("tool list: re-listed, the server's catalogue digest changed")),
        "the frame does not say the digest changed: {f:?}"
    );
}
