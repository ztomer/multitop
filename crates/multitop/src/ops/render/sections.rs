//! The Ops view's sections: what each one says, and the words it says it
//! in.
//!
//! Every section reads the server's answer and draws it. A section that has
//! nothing says so quietly - never a green "none", which is a claim about
//! nothing (an emptied Docker host read "0 running, all healthy",
//! 2026-10-10) - and a section the server does not offer says the server's
//! own reason for it.

use std::fmt::Write as _;

use super::super::model::{
    amount, load_text, memory_amount, Alert, Check, Container, Disk, Egress, Health, Job, Pool,
    Tunnel,
};
use super::super::status::{DockerHealth, DockerState, Severity, Verdict};
use super::super::{Part, Snapshot};
use super::{age, count, not_ready, Frame, Tone};

/// The share used at which a filesystem is a fault, as the estate's card
/// has it: nine tenths.
const TIGHT_PERCENT: u64 = 90;

/// Every section, in the order they are drawn.
pub(super) fn sections(f: &mut Frame<'_>, s: &Snapshot, now: i64) {
    machine(f, s);
    if !not_ready(f, "health", &s.health) {
        if let Part::Ready(h) = &s.health {
            health(f, h, now);
        }
    }
    if !not_ready(f, "vpn", &s.vpn) {
        if let Part::Ready(e) = &s.vpn {
            let containers = match &s.containers {
                Part::Ready(c) => c.as_slice(),
                Part::Absent(_) | Part::Failed(_) => &[],
            };
            vpn(f, e, containers);
        }
    }
    if !not_ready(f, "cron", &s.cron) {
        if let Part::Ready(j) = &s.cron {
            cron(f, j, now);
        }
    }
    if !not_ready(f, "containers", &s.containers) {
        if let Part::Ready(c) = &s.containers {
            containers(f, c);
        }
    }
    if !not_ready(f, "zfs", &s.zfs) {
        if let Part::Ready(p) = &s.zfs {
            zfs(f, p);
        }
    }
    if !not_ready(f, "disk", &s.disk) {
        if let Part::Ready(d) = &s.disk {
            disk(f, d, s);
        }
    }
    if !not_ready(f, "alerts", &s.alerts) {
        if let Part::Ready(a) = &s.alerts {
            alerts(f, a, now);
        }
    }
}

/// The machine's own facts: its CPUs, its load and its memory, flat and
/// unlabelled, the way the card keeps them.
///
/// A machine the server does not describe says so, with the server's reason:
/// "no cpu or memory" is a stated absence, and never a zero.
fn machine(f: &mut Frame<'_>, s: &Snapshot) {
    match &s.facts {
        Part::Ready(facts) => {
            let mut parts: Vec<String> = Vec::new();
            if facts.ncpu > 0 {
                parts.push(format!("{} cores", facts.ncpu));
            }
            if let Some(load) = facts.load.first() {
                parts.push(format!("load {}", load_text(*load)));
            }
            if facts.mem.is_reported() {
                parts.push(format!(
                    "mem {} of {} free",
                    memory_amount(facts.mem.avail_kib),
                    memory_amount(facts.mem.total_kib)
                ));
            }
            if !parts.is_empty() {
                f.note(&parts.join(" · "));
            }
            // The machine's stacks, and what is wrong with them.
            //
            // A machine that DECLARES NOTHING has one instance - itself -
            // and says so: that is a stated absence, not a fault, and alarm
            // is reserved for faults. A declaration that EXISTS and could
            // not be read is a fault (the operator declared instances and
            // this view cannot see them), and so is a declared root that is
            // not there.
            if let Some(instances) = &facts.instances {
                if instances.is_undeclared() {
                    let name = instances.primary().map_or("", |r| r.instance.as_str());
                    f.line(
                        Tone::Quiet,
                        "stacks",
                        &format!(
                            "{name}: {}, no declaration",
                            count(instances.answered, "instance")
                        ),
                    );
                }
                for row in instances.faults() {
                    let why = row.problem.as_deref().unwrap_or_default();
                    f.line(Tone::Warn, "stacks", &format!("{}: {why}", row.instance));
                }
            }
        }
        Part::Absent(why) => f.note(&format!("· no cpu or memory: {why}")),
        Part::Failed(why) => f.note(&format!("· no cpu or memory: could not read: {why}")),
    }
}

/// A section with nothing in it says so, quietly. A green "0 ok" or
/// "all healthy" over an empty set is a claim about nothing.
fn nothing(f: &mut Frame<'_>, label: &str, text: &str) {
    f.line(Tone::Quiet, label, text);
}

/// The machine's health: the PUBLISHED verdict, and how old it is.
///
/// The verdict is read, not run (servers ROADMAP E12), so it is as old as
/// the estate's last suite run - and the age is on the line, because a
/// verdict that cannot be dated is a claim nobody can check. A verdict that
/// is too old says so; a machine that published none says so; neither is
/// ever an all-clear.
fn health(f: &mut Frame<'_>, health: &Health, _now: i64) {
    if health.checks.is_empty() {
        f.line(
            Tone::Warn,
            "health",
            "no verdict published: nothing has run on this machine",
        );
        return;
    }
    let age = health.oldest_age.as_deref().unwrap_or("of unknown age");
    let bad: Vec<&Check> = health.checks.iter().filter(|c| !c.ok).collect();
    if bad.is_empty() {
        let stale = health.is_stale();
        let detail = if stale {
            format!("{} ok · {age} · stale", count(health.checks.len(), "check"))
        } else {
            format!("{} ok · {age}", count(health.checks.len(), "check"))
        };
        f.line(
            if stale { Tone::Warn } else { Tone::Good },
            "health",
            &detail,
        );
        return;
    }
    f.line(
        Tone::Bad,
        "health",
        &format!("{} of {} failing · {age}", bad.len(), health.checks.len()),
    );
    for c in bad {
        f.item(&format!(
            "{}  rc {}, {}",
            check_name(&c.name),
            c.rc,
            c.age.as_deref().unwrap_or("of unknown age")
        ));
    }
}

fn check_name(name: &str) -> &str {
    name.strip_prefix("native:").unwrap_or(name)
}

/// The machine's cron: every instance's jobs, and the trouble under the
/// stack that owns it.
fn cron(f: &mut Frame<'_>, jobs: &[Job], now: i64) {
    if jobs.is_empty() {
        nothing(f, "cron", "no jobs");
        return;
    }
    let bad: Vec<&Job> = jobs.iter().filter(|j| j.in_trouble()).collect();
    let running: Vec<String> = jobs
        .iter()
        .filter(|j| j.running_since.is_some())
        .map(job_label)
        .collect();
    let busy = if running.is_empty() {
        String::new()
    } else {
        format!(", running: {}", running.join(" "))
    };
    if bad.is_empty() {
        f.line(
            Tone::Good,
            "cron",
            &format!("{} ok{busy}", count(jobs.len(), "job")),
        );
        return;
    }
    f.line(
        Tone::Bad,
        "cron",
        &format!("{} of {} failing{busy}", bad.len(), jobs.len()),
    );
    for j in bad {
        let mut why = Vec::new();
        if let Some(rc) = j.last_rc.filter(|rc| *rc != 0) {
            let when = j
                .last_start
                .map(|t| format!(", {} ago", age(t, now)))
                .unwrap_or_default();
            why.push(format!("rc {rc}{when}"));
        }
        if let Some(t) = j.alert_since {
            why.push(format!("alerting {}", age(t, now)));
        }
        f.item(&format!("{}  {}", job_label(j), why.join(", ")));
    }
}

/// A job's name, with the stack that schedules it: two roots on one machine
/// may schedule the same label, and a job that does not say which root owns
/// it cannot be told apart from its namesake.
fn job_label(j: &Job) -> String {
    j.instance
        .as_ref()
        .map_or_else(|| j.label.clone(), |i| format!("{i} {}", j.label))
}

/// The machine's containers, one row per stack.
///
/// The stack's name is the row's subject: the machine may run several
/// roots, and a count that does not say which stack it counts is a count
/// about nothing.
fn containers(f: &mut Frame<'_>, cs: &[Container]) {
    if cs.is_empty() {
        nothing(f, "containers", "no containers");
        return;
    }
    for (stack, group) in stacks(cs) {
        stack_row(f, &stack, &group);
    }
}

/// The containers of each stack, in the order the server listed them.
fn stacks(cs: &[Container]) -> Vec<(String, Vec<&Container>)> {
    let mut out: Vec<(String, Vec<&Container>)> = Vec::new();
    for c in cs {
        let key = c.stack().to_string();
        match out.iter_mut().find(|(k, _)| *k == key) {
            Some((_, group)) => group.push(c),
            None => out.push((key, vec![c])),
        }
    }
    out
}

/// One stack's containers.
fn stack_row(f: &mut Frame<'_>, stack: &str, group: &[&Container]) {
    let bad: Vec<&&Container> = group.iter().filter(|c| c.in_trouble()).collect();
    let running = group
        .iter()
        .filter(|c| c.state == DockerState::Running)
        .count();
    // A running container that defines no Docker health check has no health
    // to report; "44 running, 33 healthy" read as 11 unhealthy when none
    // were (.33, 2026-09-23). It is named for what it is - and it is a gap
    // (nothing would notice it hang), so the line warns.
    let unchecked = group
        .iter()
        .filter(|c| c.state == DockerState::Running && c.health.is_unchecked())
        .count();
    let name = if stack.is_empty() {
        String::new()
    } else {
        format!("{stack} ")
    };
    if bad.is_empty() && running == 0 {
        // Only containers that exited cleanly, or none at all.
        nothing(
            f,
            "containers",
            &format!("{name}none running, {} stopped", group.len()),
        );
        return;
    }
    if bad.is_empty() {
        if unchecked == 0 {
            f.line(
                Tone::Good,
                "containers",
                &format!("{name}{running} running"),
            );
        } else {
            f.line(
                Tone::Warn,
                "containers",
                &format!("{name}{running} running · {unchecked} unchecked"),
            );
        }
        return;
    }
    f.line(
        Tone::Bad,
        "containers",
        &format!("{name}{} of {} in trouble", bad.len(), group.len()),
    );
    for c in bad {
        let why = if c.health == DockerHealth::Unhealthy {
            "unhealthy"
        } else {
            c.status.as_str()
        };
        f.item(&format!("{}  {why}", c.name));
    }
}

/// The machine's tunnels: where they exit, whether they leaked, and which
/// stack declares each one.
///
/// The tunnel is the estate's most consequential fact and the one no client
/// drew until now (2026-10-20): a tunnel in the wrong country that every
/// view called "fine" (2026-10-19). The stack's name is beside it, because
/// a tunnel nobody can attribute is a tunnel nobody can fix.
fn vpn(f: &mut Frame<'_>, e: &Egress, cs: &[Container]) {
    if e.tunnels.is_empty() {
        let why = if e.line.is_empty() {
            "no tunnel on this machine"
        } else {
            e.line.as_str()
        };
        nothing(f, "vpn", why);
        return;
    }
    let worst = e.tunnels.iter().max_by_key(|t| t.severity());
    let tone = match worst.map_or(Severity::Ok, Tunnel::severity) {
        Severity::Ok => Tone::Good,
        Severity::Unsettled => Tone::Warn,
        Severity::Bad => Tone::Bad,
    };
    let confirmed = e.tunnels.iter().filter(|t| t.is_confirmed()).count();
    let text = if confirmed == e.tunnels.len() {
        format!("{} confirmed", count(e.tunnels.len(), "tunnel"))
    } else {
        let word = worst.map_or_default(|t| t.leak.as_str());
        let how_many = e.tunnels.iter().filter(|t| t.leak.as_str() == word).count();
        format!("{}, {how_many} {word}", count(e.tunnels.len(), "tunnel"))
    };
    f.line(tone, "vpn", &text);
    for t in &e.tunnels {
        f.item(&tunnel_line(t, cs));
    }
}

/// One tunnel: its container, the country it exits, the verdict, and the
/// stack that declares it.
///
/// The stack is the server's `instance`. A server that does not resolve the
/// instance is answered with the compose project of the tunnel's OWN
/// container - the server's own marker, read from the server's own
/// container list, and the estate's stacks are named after their roots.
fn tunnel_line(t: &Tunnel, cs: &[Container]) -> String {
    let mut s = t.container.clone();
    if let Some(country) = &t.country {
        s.push(' ');
        s.push_str(country);
    }
    s.push(' ');
    s.push_str(t.leak.as_str());
    if let Some(instance) = t.instance.as_deref().or_else(|| stack_of(t, cs)) {
        s.push_str(" via ");
        s.push_str(instance);
    }
    s
}

/// The stack the tunnel's container belongs to, from the machine's own
/// container list.
fn stack_of<'a>(t: &Tunnel, cs: &'a [Container]) -> Option<&'a str> {
    cs.iter()
        .find(|c| c.name == t.container)
        .map(Container::stack)
        .filter(|s| !s.is_empty())
}

/// The machine's ZFS pools, as `zfs.health` published them.
///
/// A pool that is not whole is the one thing here that must be visible: a
/// SUSPENDED pool's I/O does not happen, so every other view of the host
/// says "fine" while the pool serves nothing. So a pool that is not whole
/// gets its own line, with the member that is not ONLINE and what ZFS says
/// to do about it; a host whose pools are all whole says so in one line and
/// names them.
///
/// The wording is "2 pools whole: pool0, pool1" and not "ONLINE": a pool
/// whose member is REMOVED says ONLINE and is not whole, and a view that
/// says ONLINE about it says the opposite of the truth.
fn zfs(f: &mut Frame<'_>, pools: &[Pool]) {
    if pools.is_empty() {
        nothing(f, "zfs", "no pools");
        return;
    }
    let bad: Vec<&Pool> = pools.iter().filter(|p| !p.is_whole()).collect();
    if bad.is_empty() {
        let names: Vec<&str> = pools.iter().map(|p| p.name.as_str()).collect();
        f.line(
            Tone::Good,
            "zfs",
            &format!(
                "{} {}: {}",
                count(pools.len(), "pool"),
                Verdict::Whole.as_str(),
                names.join(", ")
            ),
        );
        return;
    }
    f.line(
        Tone::Bad,
        "zfs",
        &format!(
            "{} of {} {}",
            bad.len(),
            pools.len(),
            Verdict::NotWhole.as_str()
        ),
    );
    for p in bad {
        let mut why = vec![p.state.as_str().to_owned()];
        for m in p.faulted_members() {
            why.push(format!("{} {}", m.name, m.state.as_str()));
        }
        f.item(&format!("{}  {}", p.name, why.join(" · ")));
        if !p.msg.is_empty() {
            f.item(&p.msg);
        }
        if !p.action.is_empty() {
            f.item(&format!("ZFS says: {}", p.action));
        }
    }
}

/// The machine's filesystems: the free space and the share used.
///
/// The ZFS fact decides the tone, as it does on the card: a pool that is
/// not whole makes the machine's disks not fine whatever the free space
/// says, because a SUSPENDED pool still has a mount entry and a plausible
/// size (2026-10-19).
fn disk(f: &mut Frame<'_>, d: &Disk, s: &Snapshot) {
    let Some(capacity) = d.total() else {
        nothing(f, "disk", "no filesystems");
        return;
    };
    let broken = match &s.zfs {
        Part::Ready(pools) => pools.iter().find(|p| !p.is_whole()),
        Part::Absent(_) | Part::Failed(_) => None,
    };
    let mut text = format!(
        "{} free · {}% used",
        amount(capacity.free),
        capacity.percent
    );
    let tone = match broken {
        Some(p) => {
            let _ = write!(text, " · {} is {}", p.name, p.state.as_str());
            Tone::Bad
        }
        None if capacity.percent >= TIGHT_PERCENT => Tone::Bad,
        None => Tone::Good,
    };
    f.line(tone, "disk", &text);
}

fn alerts(f: &mut Frame<'_>, events: &[Alert], now: i64) {
    if events.is_empty() {
        f.line(Tone::Good, "alerts", "none in 24 h");
        return;
    }
    f.line(Tone::Warn, "alerts", &format!("{} in 24 h", events.len()));
    let mut newest: Vec<&Alert> = events.iter().collect();
    newest.sort_by_key(|a| std::cmp::Reverse(a.ts));
    for a in newest {
        f.item(&format!("{} ago  {}", age(a.ts, now), a.subject));
    }
}
