//! A reading made by an `mcp_host` that predates the host's newest deploy
//! (queue MON.STALE).
//!
//! The Ops poll keeps ONE session per host open across polls, so the
//! `mcp_host` that answers is the process the session started - and a
//! deploy replaces the binary on disk without stopping it. On 2026-10-26 a
//! frame read `mcp-host 1.103.0` from a host that had been deployed to
//! 1.103.1: the age was printed, and nothing said the build that answered
//! was no longer the host's. `host.facts` reads the checkout's `VERSION`
//! and `head` fresh on every call, so the same answer that carries the old
//! build's version carries the new deploy's too.

use serde_json::json;

use super::render::render;
use super::tests::{pal, plain, snapshot};
use super::*;

const NOW: i64 = 1_790_170_000;

/// `host.facts` from a machine whose checkout is at `version`.
fn facts(version: &str) -> Part<Facts> {
    Part::Ready(
        parse_facts(&json!({
            "ncpu": 4,
            "load": [0.07, 0.05, 0.01],
            "mem": {"total_kib": 16_261_844, "avail_kib": 13_717_396},
            "repo": {"instance": "storage-server",
                     "head": "23dfae4e2eaa5627b2513fdde40c166bc538868a",
                     "version": version},
        }))
        .unwrap(),
    )
}

/// A snapshot answered by `mcp-host <running>` on a checkout at `deployed`.
fn answered(running: &str, deployed: &str) -> Snapshot {
    let mut s = snapshot();
    s.server = ("mcp-host".into(), running.into());
    s.facts = facts(deployed);
    s
}

fn frame(state: &OpsState) -> Vec<String> {
    plain(&render(
        "storage (192.168.0.90)",
        state,
        120,
        60,
        NOW,
        &pal(),
    ))
}

#[test]
fn the_checkout_the_host_reports_is_read() {
    let Part::Ready(f) = facts("1.103.1") else {
        panic!()
    };
    let repo = f.repo.expect("the repo block is read");
    assert_eq!(repo.version, "1.103.1");
    assert_eq!(repo.head, "23dfae4e2eaa5627b2513fdde40c166bc538868a");
    assert_eq!(repo.instance.as_deref(), Some("storage-server"));
}

#[test]
fn a_reading_by_the_build_before_the_deploy_is_stale_and_names_the_deploy() {
    let s = answered("1.103.0", "1.103.1");
    assert!(s.predates_deploy().is_some());
    let f = frame(&OpsState::Ready(Box::new(s)));
    let stale: Vec<&String> = f.iter().filter(|l| l.contains("stale")).collect();
    assert_eq!(stale.len(), 1, "one stale line: {f:?}");
    let line = stale[0];
    // The glyph and the word carry it, never the colour alone.
    assert!(line.starts_with("⚠ stale"), "{line}");
    assert!(
        line.contains("1.103.1") && line.contains("23dfae4e"),
        "the deploy is named, version and commit: {line}"
    );
    assert!(line.contains("1.103.0"), "the build that answered: {line}");
}

#[test]
fn a_reading_by_the_deployed_build_is_not_marked() {
    let s = answered("1.103.1", "1.103.1");
    assert!(s.predates_deploy().is_none());
    let f = frame(&OpsState::Ready(Box::new(s)));
    assert!(!f.iter().any(|l| l.contains("stale")), "{f:?}");
}

#[test]
fn a_host_that_does_not_report_its_checkout_is_not_guessed_at() {
    // No `host.facts`, or no `repo` block: nothing says a deploy landed, so
    // nothing is claimed either way.
    let s = snapshot();
    assert!(s.predates_deploy().is_none());
    let mut s = answered("1.103.0", "1.103.1");
    s.facts = Part::Ready(parse_facts(&json!({"ncpu": 4})).unwrap());
    assert!(s.predates_deploy().is_none());
}

#[test]
fn the_last_answer_under_a_failure_is_marked_too() {
    let state = OpsState::Failed {
        why: "mcp_host closed the session".into(),
        last: Some(Box::new(answered("1.103.0", "1.103.1"))),
    };
    let f = frame(&state);
    assert!(f.iter().any(|l| l.starts_with("⚠ stale")), "{f:?}");
}
