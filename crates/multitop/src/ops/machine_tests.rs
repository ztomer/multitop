//! The machine model in the Ops view: the tunnel and the stack that
//! declares it, the machine's capacity, and the containers grouped by the
//! stack they belong to.
//!
//! The tunnel is the fact the whole rework is for. On 2026-10-19 a tunnel
//! in Zurich was "fine" in every view, because no view drew the tunnel at
//! all - and a tunnel nobody can attribute to a stack is a tunnel nobody
//! can fix.

use serde_json::{json, Value};

use super::render::render;
use super::tests::{pal, plain};
use super::*;
use crate::mcp::client::{Client, PROTOCOL_VERSION};
use crate::mcp::fake::{discover, serve, tools, Reply};

const NOW: i64 = 1_790_170_000;

/// `net.egress` as the machine-scoped server answers it: one tunnel,
/// declared by the solver stack.
fn egress_answer() -> Value {
    json!({
        "country": "CH",
        "egress": "CH",
        "leak": "confirmed",
        "tunnel": "solver-vpn",
        "expected": "CH",
        "gluetun_running": true,
        "children": [],
        "resolvers": ["127.0.0.53"],
        "tunnels": [{
            "instance": "solver",
            "container": "solver-vpn",
            "country": "CH",
            "egress": "CH",
            "leak": "confirmed",
            "gluetun_running": true,
            "children": [],
        }],
    })
}

/// `disk.usage` as the storage machine answers it: two pools' datasets and
/// one boot partition.
fn disk_answer() -> Value {
    json!({
        "mounts": [
            {"mount": "/", "device": "rpool/ROOT/ubuntu", "fstype": "zfs",
             "size": 488_844_689_408_u64, "used": 3_549_822_976_u64,
             "avail": 485_294_866_432_u64, "percent": 1},
            {"mount": "/pool0", "device": "pool0", "fstype": "zfs",
             "size": 4_336_810_000_000_u64, "used": 45_000_000_u64,
             "avail": 4_336_765_000_000_u64, "percent": 1},
            {"mount": "/boot", "device": "bpool/BOOT/ubuntu", "fstype": "zfs",
             "size": 1_812_402_688_u64, "used": 241_172_480_u64,
             "avail": 1_639_972_864_u64, "percent": 13},
            {"mount": "/boot/efi", "device": "/dev/sda1", "fstype": "vfat",
             "size": 536_870_912_u64, "used": 17_694_720_u64,
             "avail": 519_839_744_u64, "percent": 3},
        ],
        "zfs": [],
        "pools": [
            {"name": "rpool", "size": "472G", "alloc": "5.41G", "free": "467G", "cap": "1%", "health": "ONLINE"},
            {"name": "pool0", "size": "3.62T", "alloc": "43.0M", "free": "3.62T", "cap": "0%", "health": "ONLINE"},
        ],
    })
}

/// `host.facts` as the machine answers it: four CPUs, its memory, and the
/// two instances it declares.
fn facts_answer() -> Value {
    json!({
        "hostname": "storage",
        "kernel": "7.0.0-38-generic",
        "arch": "x86_64",
        "uptime_s": 54_745,
        "ncpu": 4,
        "load": [0.07, 0.05, 0.01],
        "mem": {"total_kib": 16_261_844, "avail_kib": 13_717_396},
        "binaries": [],
        "repo": {"head": "bb82c859", "version": "1.99.6"},
        "instances": {
            "source": "/home/ztomer/prj/storage-server/config/instances.tsv",
            "mtime": 1_790_000_000,
            "declared": 2,
            "read": true,
            "answered": 2,
            "rows": [
                {"instance": "storage-server", "root": "/home/ztomer/prj/storage-server",
                 "present": true, "problem": null},
                {"instance": "solver", "root": "/home/ztomer/prj/solver",
                 "present": true, "problem": null},
            ],
        },
    })
}

/// The containers as the machine answers them: four in the storage stack,
/// three in the solver stack.
fn containers_answer() -> Value {
    json!({"containers": [
        {"name": "navidrome", "state": "running", "status": "Up 45 minutes",
         "health": "healthy", "instance": "storage-server", "compose_project": "storage-server"},
        {"name": "solver-vpn", "state": "running", "status": "Up 15 hours",
         "health": "healthy", "instance": "solver", "compose_project": "solver"},
        {"name": "solver-trawl", "state": "exited", "status": "Exited (137) 5 minutes ago",
         "health": "none", "instance": "solver", "compose_project": "solver"},
    ]})
}

/// A snapshot with every section the machine answered.
fn snapshot() -> Snapshot {
    Snapshot {
        server: ("mcp-host".into(), "1.99.6".into()),
        at: NOW - 12,
        sampled_every_s: FALLBACK_INTERVAL_S,
        health: Part::Ready(Health {
            cadence: Cadence::default(),
            checks: vec![Check {
                name: "native:check_vpn_leak".into(),
                instance: Some("media_server".into()),
                ok: true,
                rc: 0,
                ts: NOW - 600,
                age: Some("10 min ago".into()),
            }],
            oldest_age: Some("10 min ago".into()),
            oldest_age_s: Some(600),
        }),
        vpn: Part::Ready(parse_egress(&egress_answer()).unwrap()),
        cron: Part::Ready(vec![]),
        containers: Part::Ready(parse_containers(&containers_answer()).unwrap()),
        zfs: Part::Ready(vec![]),
        disk: Part::Ready(parse_disk(&disk_answer()).unwrap()),
        facts: Part::Ready(parse_facts(&facts_answer()).unwrap()),
        alerts: Part::Ready(vec![]),
    }
}

/// The frame of `snapshot`, as plain lines.
fn frame(s: &Snapshot) -> Vec<String> {
    plain(&render(
        "storage (192.168.0.90)",
        &OpsState::Ready(Box::new(s.clone())),
        100,
        40,
        NOW,
        &pal(),
    ))
}

#[test]
fn the_tunnel_is_drawn_with_the_stack_that_declares_it() {
    let f = frame(&snapshot());
    assert!(
        f.contains(&"✓ vpn        1 tunnel confirmed".to_string()),
        "{f:?}"
    );
    assert!(
        f.contains(&"    solver-vpn CH confirmed via solver".to_string()),
        "the stack that declares the tunnel is named beside it: {f:?}"
    );
}

#[test]
fn a_leaked_tunnel_is_a_fault_and_says_where_it_exited() {
    let mut s = snapshot();
    let mut answer = egress_answer();
    answer["tunnels"][0]["leak"] = json!("leaked");
    answer["tunnels"][0]["country"] = json!(null);
    answer["leak"] = json!("leaked");
    answer["egress"] = json!("BR (POSSIBLE LEAK)");
    s.vpn = Part::Ready(parse_egress(&answer).unwrap());
    let f = frame(&s);
    assert!(
        f.contains(&"✗ vpn        1 tunnel, 1 leaked".to_string()),
        "{f:?}"
    );
    assert!(
        f.contains(&"    solver-vpn leaked via solver".to_string()),
        "{f:?}"
    );
}

#[test]
fn a_machine_with_no_tunnel_says_so_with_the_servers_own_reason() {
    // The server does not list `net.egress`, and says why in its
    // `dev.multitop/absent` meta: an absence with a reason, never a green
    // tick and never an empty section.
    let mut s = snapshot();
    s.vpn = Part::Absent("no instance of this machine declares a VPN-namespace tunnel".into());
    let f = frame(&s);
    assert!(
        f.contains(
            &"· vpn        no instance of this machine declares a VPN-namespace tunnel".to_string()
        ),
        "{f:?}"
    );
    assert!(
        !f.iter().any(|l| l.starts_with("✓ vpn")),
        "an absent tunnel is never a green tick: {f:?}"
    );
}

#[test]
fn a_server_that_does_not_name_the_instance_is_answered_from_its_own_container_list() {
    // The tunnel's stack is the server's `instance`. A server that does not
    // resolve it is answered with the compose project of the tunnel's own
    // container - the server's own marker, and the estate's stacks are
    // named after their roots.
    let mut s = snapshot();
    let mut answer = egress_answer();
    answer["tunnels"][0]
        .as_object_mut()
        .unwrap()
        .remove("instance");
    s.vpn = Part::Ready(parse_egress(&answer).unwrap());
    let f = frame(&s);
    assert!(
        f.contains(&"    solver-vpn CH confirmed via solver".to_string()),
        "{f:?}"
    );
}

#[test]
fn the_containers_are_grouped_by_the_stack_they_belong_to() {
    let f = frame(&snapshot());
    assert!(
        f.contains(&"✓ containers storage-server 1 running".to_string()),
        "one row per stack, with its count: {f:?}"
    );
    assert!(
        f.contains(&"✗ containers solver 1 of 2 in trouble".to_string()),
        "{f:?}"
    );
    assert!(
        f.contains(&"    solver-trawl  Exited (137) 5 minutes ago".to_string()),
        "the container in trouble is named under its stack: {f:?}"
    );
}

#[test]
fn the_disk_line_says_how_full_the_machine_is() {
    // The pool's datasets share one pool's space, so the pool counts once:
    // the total is the machine's, not its datasets' sum.
    let f = frame(&snapshot());
    assert!(
        f.contains(&"✓ disk       4.4 TB free · 0% used".to_string()),
        "{f:?}"
    );
}

#[test]
fn the_machine_gets_its_name_beside_its_address() {
    let server = crate::config::Server {
        host: "192.168.0.90".into(),
        name: Some("storage".into()),
        ..crate::config::Server::default()
    };
    assert_eq!(banner(&server), "storage (192.168.0.90)");
    let unnamed = crate::config::Server {
        host: "192.168.0.90".into(),
        ..crate::config::Server::default()
    };
    assert_eq!(banner(&unnamed), "192.168.0.90");
}

#[test]
fn the_words_are_the_ones_the_other_client_uses() {
    let mut s = snapshot();
    s.zfs = Part::Ready(vec![
        Pool {
            name: "pool0".into(),
            state: ZfsState::Online,
            action: String::new(),
            msg: String::new(),
            members: vec![Member {
                name: "mirror-0".into(),
                state: ZfsState::Online,
            }],
        },
        Pool {
            name: "pool1".into(),
            state: ZfsState::Online,
            action: String::new(),
            msg: String::new(),
            members: vec![Member {
                name: "sdb".into(),
                state: ZfsState::Online,
            }],
        },
    ]);
    let f = frame(&s);
    assert!(
        f.contains(&"✓ zfs        2 pools whole: pool0, pool1".to_string()),
        "the settled wording, and not the word that lies: {f:?}"
    );
    for word in ["containers", "vpn", "disk", "zfs", "cron", "health"] {
        assert!(
            f.iter().any(|l| l.contains(&format!("{word} "))),
            "the section {word} is named as the other client names it: {f:?}"
        );
    }
    assert!(
        !f.iter().any(|l| l.contains("zfs.health")),
        "the tool's name is not the section's name: {f:?}"
    );
    assert!(
        !f.iter().any(|l| l.contains("docker")),
        "the section is `containers`, as the other client calls it: {f:?}"
    );
}

#[test]
fn the_capacity_is_the_machines_own() {
    let f = frame(&snapshot());
    assert!(
        f.iter().any(|l| l.contains("4 cores")
            && l.contains("load 0.07")
            && l.contains("mem 13 GB of 16 GB free")),
        "the machine's CPUs, its load and its memory: {f:?}"
    );
}

#[test]
fn a_declared_root_that_is_not_there_is_a_finding() {
    let mut answer = facts_answer();
    answer["instances"]["rows"][1]["present"] = json!(false);
    answer["instances"]["rows"][1]["problem"] =
        json!("the root `/home/ztomer/prj/solver` is not there");
    let mut s = snapshot();
    s.facts = Part::Ready(parse_facts(&answer).unwrap());
    let f = frame(&s);
    assert!(
        f.contains(
            &"⚠ stacks     solver: the root `/home/ztomer/prj/solver` is not there".to_string()
        ),
        "a declared root that is gone is named, with the server's reason: {f:?}"
    );
}

/// A machine with no declaration file has ONE instance - itself - and says
/// so. That is a stated absence, not a fault: alarm is reserved for faults,
/// and a healthy single-instance machine is not one.
#[test]
fn a_machine_with_no_declaration_says_so_and_is_not_an_alarm() {
    let mut answer = facts_answer();
    answer["instances"] = json!({
        "source": "/mnt/fast-nvme/projects/media_server/config/instances.tsv",
        "mtime": Value::Null,
        "declared": 0,
        "read": false,
        "answered": 1,
        "rows": [{
            "instance": "media_server",
            "root": "/mnt/fast-nvme/projects/media_server",
            "present": true,
            "problem": "the machine's instance declaration is missing (config/instances.tsv)",
        }],
    });
    let mut s = snapshot();
    s.facts = Part::Ready(parse_facts(&answer).unwrap());
    let f = frame(&s);
    assert!(
        f.contains(&"· stacks     media_server: 1 instance, no declaration".to_string()),
        "the absence is stated, quietly: {f:?}"
    );
    assert!(
        !f.iter().any(|l| l.starts_with("⚠ stacks")),
        "a machine that simply declares no instances is not a fault: {f:?}"
    );
}

/// A declaration that EXISTS and could not be read is a different thing:
/// the operator declared instances and this view cannot see them, so it is
/// a fault, and it says so.
#[test]
fn a_declaration_that_could_not_be_read_is_a_fault() {
    let mut answer = facts_answer();
    answer["instances"] = json!({
        "source": "/mnt/fast-nvme/projects/media_server/config/instances.tsv",
        "mtime": 1_790_000_000,
        "declared": 0,
        "read": false,
        "answered": 1,
        "rows": [{
            "instance": "media_server",
            "root": "/mnt/fast-nvme/projects/media_server",
            "present": true,
            "problem": "the machine's instance declaration could not be read (config/instances.tsv)",
        }],
    });
    let mut s = snapshot();
    s.facts = Part::Ready(parse_facts(&answer).unwrap());
    let f = frame(&s);
    assert!(
        f.iter().any(|l| l.starts_with(
            "⚠ stacks     media_server: the machine's instance declaration could not be read"
        )),
        "a broken declaration is a fault, with the server's reason: {f:?}"
    );
}

/// The fake host the estate's server is: every tool the machine lists.
fn host(tool_names: &'static [&'static str]) -> impl Fn(&str, &Value) -> Reply + Send + 'static {
    move |method, params| match method {
        "server/discover" => Reply::Result(discover(&[PROTOCOL_VERSION])),
        "tools/list" => Reply::Result(tools(tool_names)),
        "resources/read" => Reply::Result(json!({
            "contents": [{"text": json!({"checks": []}).to_string()}]
        })),
        "tools/call" => match params["name"].as_str().unwrap_or_default() {
            "net.egress" => Reply::Result(json!({"structuredContent": egress_answer()})),
            "disk.usage" => Reply::Result(json!({"structuredContent": disk_answer()})),
            "host.facts" => Reply::Result(json!({"structuredContent": facts_answer()})),
            _ => Reply::Error(-32602, "unknown"),
        },
        _ => Reply::Error(-32601, "no"),
    }
}

#[tokio::test]
async fn the_poll_asks_the_tunnel_the_capacity_and_the_facts() {
    let (rd, wr, seen) = serve(host(&["net.egress", "disk.usage", "host.facts"]));
    let mut client = Client::connect(rd, wr, std::time::Duration::from_secs(2))
        .await
        .unwrap();
    let s = gather(&mut client, NOW, Previous::default()).await;
    assert!(matches!(s.vpn, Part::Ready(ref v) if v.tunnels.len() == 1));
    assert!(matches!(s.disk, Part::Ready(ref d) if d.mounts.len() == 4));
    assert!(matches!(s.facts, Part::Ready(ref f) if f.ncpu == 4));
    let asked: Vec<String> = seen
        .lock()
        .unwrap()
        .iter()
        .filter_map(|r| r["params"]["name"].as_str().map(str::to_string))
        .collect();
    assert!(asked.contains(&"net.egress".to_string()), "{asked:?}");
    assert!(asked.contains(&"disk.usage".to_string()), "{asked:?}");
    assert!(asked.contains(&"host.facts".to_string()), "{asked:?}");
    // A machine that does not list the tunnel says why, in its own words.
    let (rd, wr, _) = serve(host(&["disk.usage"]));
    let mut client = Client::connect(rd, wr, std::time::Duration::from_secs(2))
        .await
        .unwrap();
    let s = gather(&mut client, NOW, Previous::default()).await;
    assert!(matches!(s.vpn, Part::Absent(_)));
}
