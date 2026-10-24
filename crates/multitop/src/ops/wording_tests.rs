//! A machine that DECLARES NOTHING is not a fault, and the Ops view's
//! words must not read as one.
//!
//! servers ROADMAP E26: `.33` and `.158` carry no `config/instances.tsv`,
//! so the server answers "the machine's instance declaration is missing
//! or unreadable ...: its instances are unknown" - and the truth is
//! "one instance: this machine, and it declares none". The routines card
//! says "1 stack: `adguard_server` · no declaration"; the monitor's `vpn`
//! line repeated the server's wording, which reads like a fault.
//!
//! E26's rule, settled: neither client uses an alarm glyph or the word
//! "unknown" for a machine that declares nothing. The monitor knows the
//! machine's instances - `host.facts` publishes them - so it says what it
//! knows.

use super::model::{Facts, InstanceRow, Instances, Memory};
use super::render::render;
use super::tests::{pal, plain};
use super::*;

const NOW: i64 = 1_790_170_000;

/// The machine's own facts: one instance - itself - and no declaration.
fn undeclared() -> Facts {
    Facts {
        ncpu: 4,
        load: vec![210],
        mem: Memory {
            total_kib: 1_800_000,
            avail_kib: 900_000,
        },
        instances: Some(Instances {
            declared: 0,
            read: false,
            mtime: None,
            answered: 1,
            rows: vec![InstanceRow {
                instance: "adguard_server".into(),
                root: "/home/ztomer/prj/adguard_server".into(),
                present: true,
                problem: Some(
                    "the machine's instance declaration is missing (config/instances.tsv)".into(),
                ),
            }],
        }),
    }
}

/// The machine's answer: the tunnel's tool is not offered, and the
/// server's reason for that reads like a fault.
const NO_TUNNEL: &str = "no Docker on this machine: no daemon socket; the machine's instance \
                         declaration is missing or unreadable \
                         (/home/ztomer/prj/adguard_server/config/instances.tsv): its instances \
                         are unknown";

/// A snapshot of a machine that declares nothing.
fn snapshot() -> Snapshot {
    Snapshot {
        server: ("mcp_host".into(), "1.99.14".into()),
        at: NOW - 12,
        sampled_every_s: 450,
        catalogue: Catalogue::Listed { ttl_s: 3_600 },
        health: Part::Absent("this host lists no health.list".into()),
        vpn: Part::Absent(NO_TUNNEL.to_owned()),
        cron: Part::Ready(Vec::new()),
        containers: Part::Absent("no Docker on this machine: no daemon socket".into()),
        zfs: Part::Absent("no ZFS on this machine: no /usr/sbin/zpool".into()),
        disk: Part::Ready(super::model::Disk { mounts: Vec::new() }),
        facts: Part::Ready(undeclared()),
        alerts: Part::Ready(Vec::new()),
        triage: Part::Absent("this host lists no machine.triage".into()),
    }
}

/// The frame for the machine, as plain lines.
fn frame() -> Vec<String> {
    plain(&render(
        "pihole",
        &OpsState::Ready(Box::new(snapshot())),
        120,
        40,
        NOW,
        &pal(),
    ))
}

/// **A MACHINE THAT DECLARES NOTHING IS NOT A FAULT.** The `vpn` line
/// must not carry the old ambiguity: no "unknown", no "missing or
/// unreadable", and no alarm.
#[test]
fn the_vpn_line_does_not_read_a_machine_with_no_declaration_as_a_fault() {
    let f = frame();
    let vpn = f
        .iter()
        .find(|l| l.contains("vpn"))
        .cloned()
        .unwrap_or_else(|| panic!("the frame draws no vpn line: {f:?}"));
    assert!(
        !vpn.contains("unknown"),
        "the vpn line still calls the machine's instances unknown: {vpn}"
    );
    assert!(
        !vpn.contains("missing or unreadable"),
        "the vpn line still says the declaration could not be read: {vpn}"
    );
    assert!(
        vpn.contains("no declaration"),
        "the vpn line does not say what it means: {vpn}"
    );
    assert!(
        !vpn.contains('✗') && !vpn.contains('⚠'),
        "a machine that declares nothing is not an alarm: {vpn}"
    );
}

/// **THE MACHINE IS NAMED.** "no declaration" without the machine is a
/// fact about nothing.
#[test]
fn the_vpn_line_names_the_machine_that_declares_nothing() {
    let f = frame();
    let vpn = f
        .iter()
        .find(|l| l.contains("vpn"))
        .cloned()
        .unwrap_or_default();
    assert!(
        vpn.contains("adguard_server"),
        "the vpn line does not name the machine: {vpn}"
    );
}

/// **THE SERVER'S REASON IS KEPT WHEN THE MONITOR DOES NOT KNOW BETTER.**
/// A machine whose declaration EXISTS and could not be read is a fault,
/// and the server's own words are the ones that say so.
#[test]
fn the_server_reason_is_kept_when_the_declaration_is_a_fault() {
    let mut s = snapshot();
    let Part::Ready(facts) = &mut s.facts else {
        panic!("the facts are ready");
    };
    let Some(instances) = &mut facts.instances else {
        panic!("the declaration is published");
    };
    // A declaration that EXISTS and could not be read: an mtime, and a
    // row the client cannot account for.
    instances.mtime = Some(1_700_000_000);
    instances.declared = 2;
    instances.rows.push(InstanceRow {
        instance: "solver".into(),
        root: "/home/ztomer/prj/solver".into(),
        present: false,
        problem: Some("the declared instance `solver` is not there".into()),
    });
    let f = plain(&render(
        "storage",
        &OpsState::Ready(Box::new(s)),
        120,
        40,
        NOW,
        &pal(),
    ));
    let vpn = f
        .iter()
        .find(|l| l.contains("vpn"))
        .cloned()
        .unwrap_or_default();
    assert!(
        vpn.contains("missing or unreadable"),
        "the server's own reason is dropped for a machine that IS in trouble: {vpn}"
    );
}
