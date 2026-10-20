//! The server's answers, parsed: what a tool said, in the shape the view
//! draws.
//!
//! Every parser here READS. It never guesses a fact the server owns: a row
//! without a field is a field the server did not publish, and the view says
//! what it does not have rather than inventing a value for it.

use serde_json::Value;

use super::model::{
    load_milli, Alert, Check, Container, Disk, Egress, Facts, InstanceRow, Instances, Job, Member,
    Memory, Mount, Pool, Tunnel,
};

/// The string at `k`, or the empty string when there is none.
fn text(v: &Value, k: &str) -> String {
    v.get(k)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}

/// The string at `k`, when it is one and is not empty.
fn opt_text(v: &Value, k: &str) -> Option<String> {
    v.get(k)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// The list at `k`.
///
/// # Errors
///
/// The answer has no such list.
fn list<'a>(v: &'a Value, k: &str) -> Result<&'a Vec<Value>, String> {
    v.get(k)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("the answer has no `{k}` list"))
}

/// The integer at `k`, or zero when there is none.
fn int(v: &Value, k: &str) -> i64 {
    v.get(k).and_then(Value::as_i64).unwrap_or_default()
}

/// The count at `k`, or zero when there is none.
fn count(v: &Value, k: &str) -> u64 {
    v.get(k).and_then(Value::as_u64).unwrap_or_default()
}

/// `health://latest`: `{checks: [{check, ok, rc, ms, ts}]}`.
///
/// # Errors
///
/// The answer has no `checks` list.
pub fn parse_health(v: &Value) -> Result<Vec<Check>, String> {
    Ok(list(v, "checks")?
        .iter()
        .map(|c| Check {
            name: text(c, "check"),
            ok: c.get("ok").and_then(Value::as_bool).unwrap_or(false),
            rc: int(c, "rc"),
            ts: int(c, "ts"),
        })
        .collect())
}

/// `cron.status`: `{jobs: [{label, instance, last_start, last_rc,
/// running_since, alert_since, ...}]}`.
///
/// # Errors
///
/// The answer has no `jobs` list.
pub fn parse_cron(v: &Value) -> Result<Vec<Job>, String> {
    let int = |j: &Value, k: &str| j.get(k).and_then(Value::as_i64);
    Ok(list(v, "jobs")?
        .iter()
        .map(|j| Job {
            label: text(j, "label"),
            instance: opt_text(j, "instance"),
            last_start: int(j, "last_start"),
            last_rc: int(j, "last_rc"),
            running_since: int(j, "running_since"),
            alert_since: int(j, "alert_since"),
        })
        .collect())
}

/// `docker.containers`: `{containers: [{name, state, status, health,
/// instance, compose_project, ...}]}`.
///
/// # Errors
///
/// The answer has no `containers` list.
pub fn parse_containers(v: &Value) -> Result<Vec<Container>, String> {
    Ok(list(v, "containers")?
        .iter()
        .map(|c| Container {
            name: text(c, "name"),
            state: text(c, "state"),
            status: text(c, "status"),
            health: text(c, "health"),
            instance: opt_text(c, "instance"),
            compose_project: opt_text(c, "compose_project"),
        })
        .collect())
}

/// `alerts.since`: `{events: [{ts, subject, ...}], marker}`. `ts` arrives
/// as a string or a number.
///
/// # Errors
///
/// The answer has no `events` list.
pub fn parse_alerts(v: &Value) -> Result<Vec<Alert>, String> {
    Ok(list(v, "events")?
        .iter()
        .map(|e| Alert {
            ts: e
                .get("ts")
                .and_then(|t| {
                    t.as_i64()
                        .or_else(|| t.as_str().and_then(|s| s.parse().ok()))
                })
                .unwrap_or_default(),
            subject: text(e, "subject"),
        })
        .collect())
}

/// `zfs.health`: `{pools: [{name, state, action, msg, scan, members: [{name,
/// state, read, write, cksum}]}]}`.
///
/// The pool's own state and its vdev tree, as the tool publishes them: the
/// view reads them, and never runs `zpool status` itself.
///
/// The error counters are not read, and that is the estate's rule rather than
/// an omission: a pool is whole when its state and every member's state say
/// so, and `check_zfs_pool` (which owns that verdict) fails on the states,
/// not on a counter over a member that is still ONLINE.
///
/// # Errors
///
/// The answer has no `pools` list, or a pool has no `members` list (a pool
/// whose vdev tree could not be read is not a pool that may be called whole).
pub fn parse_zfs(v: &Value) -> Result<Vec<Pool>, String> {
    list(v, "pools")?
        .iter()
        .map(|p| {
            let members = list(p, "members").map_err(|_| {
                format!(
                    "pool {} has no `members` list",
                    p.get("name").and_then(Value::as_str).unwrap_or_default()
                )
            })?;
            Ok(Pool {
                name: text(p, "name"),
                state: text(p, "state"),
                action: text(p, "action"),
                msg: text(p, "msg"),
                members: members
                    .iter()
                    .map(|m| Member {
                        name: text(m, "name"),
                        state: text(m, "state"),
                    })
                    .collect(),
            })
        })
        .collect()
}

/// `net.egress`: `{tunnels: [{instance, container, country, egress, leak,
/// children}], ...}`.
///
/// The machine's tunnels, one row per instance that declares one. A server
/// that has not been reworked for the machine model answers a single tunnel
/// under the shorter name (`tunnel`, `country`, `leak`); that tunnel is read
/// too, and its instance is left unknown, because the server did not name
/// one.
///
/// # Errors
///
/// Never: the answer is read as far as it goes.
pub fn parse_egress(v: &Value) -> Result<Egress, String> {
    let line = text(v, "egress");
    if let Some(rows) = v.get("tunnels").and_then(Value::as_array) {
        return Ok(Egress {
            tunnels: rows.iter().map(tunnel).collect(),
            line,
        });
    }
    let container = text(v, "tunnel");
    let tunnels = if container.is_empty() {
        Vec::new()
    } else {
        vec![Tunnel {
            instance: None,
            container,
            country: opt_text(v, "country"),
            leak: text(v, "leak"),
        }]
    };
    Ok(Egress { tunnels, line })
}

/// One row of the `tunnels` list.
fn tunnel(row: &Value) -> Tunnel {
    Tunnel {
        instance: opt_text(row, "instance"),
        container: text(row, "container"),
        country: opt_text(row, "country"),
        leak: text(row, "leak"),
    }
}

/// `disk.usage`: `{mounts: [{mount, device, fstype, size, used, avail,
/// percent}], ...}`.
///
/// The machine's filesystems. The ZFS pools the answer also carries are not
/// read: the view counts the pools through their mounts, which is where the
/// space is, and a pool's own row would count the same space twice.
///
/// # Errors
///
/// The answer has no `mounts` list.
pub fn parse_disk(v: &Value) -> Result<Disk, String> {
    Ok(Disk {
        mounts: list(v, "mounts")?
            .iter()
            .map(|m| Mount {
                mount: text(m, "mount"),
                device: text(m, "device"),
                fstype: text(m, "fstype"),
                size: count(m, "size"),
                avail: count(m, "avail"),
                percent: count(m, "percent"),
            })
            .collect(),
    })
}

/// `host.facts`: `{hostname, ncpu, load, mem: {total_kib, avail_kib},
/// instances: {...}}`.
///
/// The machine's identity and its capacity, and the instances it declares.
///
/// # Errors
///
/// Never: the answer is read as far as it goes.
pub fn parse_facts(v: &Value) -> Result<Facts, String> {
    let mem = v.get("mem");
    let memory = Memory {
        total_kib: mem.map_or(0, |m| count(m, "total_kib")),
        avail_kib: mem.map_or(0, |m| count(m, "avail_kib")),
    };
    let instances = v
        .get("instances")
        .filter(|i| i.is_object())
        .map(parse_instances);
    Ok(Facts {
        ncpu: int(v, "ncpu"),
        load: v
            .get("load")
            .and_then(Value::as_array)
            .map(|l| l.iter().filter_map(Value::as_f64).map(load_milli).collect())
            .unwrap_or_default(),
        mem: memory,
        instances,
    })
}

/// The machine's `instances` block: the declaration it published.
fn parse_instances(v: &Value) -> Instances {
    let rows = v
        .get("rows")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .map(|r| InstanceRow {
                    instance: text(r, "instance"),
                    root: text(r, "root"),
                    present: r.get("present").and_then(Value::as_bool).unwrap_or(false),
                    problem: opt_text(r, "problem"),
                })
                .collect()
        })
        .unwrap_or_default();
    Instances {
        declared: v
            .get("declared")
            .and_then(Value::as_u64)
            .and_then(|d| usize::try_from(d).ok())
            .unwrap_or_default(),
        read: v.get("read").and_then(Value::as_bool).unwrap_or(false),
        mtime: v.get("mtime").and_then(Value::as_i64),
        answered: v
            .get("answered")
            .and_then(Value::as_u64)
            .and_then(|a| usize::try_from(a).ok())
            .unwrap_or_default(),
        rows,
    }
}
