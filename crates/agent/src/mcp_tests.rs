//! The server, over a fake machine: what it lists, what it answers, and the
//! protocol both kinds of client speak.

use std::cell::RefCell;

use serde_json::{json, Value};

use super::*;
use crate::docker_transport::DockerEndpoint;

struct Fake {
    launchd: Option<&'static str>,
    labels: Vec<String>,
    docker: Option<DockerEndpoint>,
    containers: Result<Value, String>,
    asked_ages: RefCell<Vec<u32>>,
}

impl Fake {
    fn mac() -> Self {
        Self {
            launchd: Some("PID\tStatus\tLabel\n42\t0\tcom.me.watch\n-\t78\tcom.me.nightly\n-\t0\tcom.apple.x\n"),
            labels: vec!["com.me.nightly".into(), "com.me.watch".into()],
            docker: Some(DockerEndpoint::Unix("/h/.colima/default/docker.sock".into())),
            containers: Ok(json!({ "containers": [{ "name": "web", "state": "running" }] })),
            asked_ages: RefCell::new(Vec::new()),
        }
    }
}

impl mcp_tools::System for Fake {
    fn launchctl_list(&self) -> Option<String> {
        self.launchd.map(str::to_string)
    }
    fn agent_labels(&self) -> Vec<String> {
        self.labels.clone()
    }
    fn ages(&self, pids: &[u32]) -> String {
        self.asked_ages.borrow_mut().extend_from_slice(pids);
        pids.iter().map(|p| p.to_string() + " 01:00\n").collect()
    }
    fn docker(&self) -> Option<DockerEndpoint> {
        self.docker.clone()
    }
    fn containers(&self, _: &DockerEndpoint) -> Result<Value, String> {
        self.containers.clone()
    }
    fn now(&self) -> i64 {
        1_000
    }
}

fn meta() -> Value {
    json!({ "io.modelcontextprotocol/protocolVersion": "2026-07-28",
            "io.modelcontextprotocol/clientCapabilities": {} })
}

fn ask(system: &Fake, method: &str, mut params: Value) -> Value {
    params["_meta"] = meta();
    let line = json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params });
    handle(&line.to_string(), system).expect("a request is answered")
}

fn names(list: &Value) -> Vec<String> {
    list["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn discover_offers_2026_and_says_why_each_unlisted_section_is_absent() {
    let d = ask(&Fake::mac(), "server/discover", json!({}));
    let r = &d["result"];
    assert!(
        r["supportedVersions"]
            .as_array()
            .unwrap()
            .contains(&json!("2026-07-28")),
        "{d}"
    );
    assert_eq!(
        r["_meta"]["io.modelcontextprotocol/serverInfo"]["name"],
        "multitop-agent"
    );
    let absent = &r["_meta"][META_ABSENT];
    assert!(
        absent["health.run"].as_str().unwrap().contains("health"),
        "{absent}"
    );
    assert!(
        absent["alerts.since"].as_str().unwrap().contains("alerts"),
        "{absent}"
    );
    assert!(
        absent["zfs.health"].as_str().unwrap().contains("ZFS"),
        "the ZFS section is named, with its reason: {absent}"
    );
    assert!(
        absent.get("cron.status").is_none(),
        "cron is listed on a Mac: {absent}"
    );
}

#[test]
fn a_host_lists_only_what_it_can_answer() {
    let mac = Fake::mac();
    assert_eq!(
        names(&ask(&mac, "tools/list", json!({}))),
        ["cron.status", "docker.containers"]
    );

    let bare = Fake {
        launchd: None,
        docker: None,
        ..Fake::mac()
    };
    multitop_testassert::assert_empty!(names(&ask(&bare, "tools/list", json!({}))));
    let absent = &ask(&bare, "server/discover", json!({}))["result"]["_meta"][META_ABSENT];
    assert!(
        absent["docker.containers"]
            .as_str()
            .unwrap()
            .contains("no Docker"),
        "{absent}"
    );
    assert!(
        absent["cron.status"].as_str().unwrap().contains("launchd"),
        "{absent}"
    );
    // A tool the listing withheld is refused, not answered with a guess.
    let r = ask(&bare, "tools/call", json!({ "name": "docker.containers" }));
    assert_eq!(r["result"]["isError"], true, "{r}");
}

#[test]
fn cron_status_is_the_users_agents_with_ages_asked_only_for_them() {
    let mac = Fake::mac();
    let r = ask(
        &mac,
        "tools/call",
        json!({ "name": "cron.status", "arguments": {} }),
    );
    assert_eq!(r["result"]["isError"], false, "{r}");
    let jobs = r["result"]["structuredContent"]["jobs"].as_array().unwrap();
    assert_eq!(
        jobs.len(),
        2,
        "Apple's labels are not the user's jobs: {jobs:?}"
    );
    assert_eq!(jobs[0]["label"], "com.me.nightly");
    assert_eq!(jobs[0]["last_rc"], 78);
    assert_eq!(jobs[1]["running_since"], 1_000 - 60);
    assert_eq!(*mac.asked_ages.borrow(), [42]);
    // The text content carries the same answer for a client without
    // structured content.
    let text: Value =
        serde_json::from_str(r["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(text, r["result"]["structuredContent"]);
}

#[test]
fn a_daemon_that_fails_is_a_tool_error_with_its_reason() {
    let down = Fake {
        containers: Err("docker at unix:///x: Connection refused".into()),
        ..Fake::mac()
    };
    let r = ask(&down, "tools/call", json!({ "name": "docker.containers" }));
    assert_eq!(r["result"]["isError"], true);
    assert!(
        r["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("refused"),
        "{r}"
    );
}

#[test]
fn the_protocol_holds_for_both_kinds_of_client() {
    let mac = Fake::mac();
    // 2026-07-28: an unknown version is -32022; the handshake does not exist.
    let mut p = json!({});
    p["_meta"] = json!({ "io.modelcontextprotocol/protocolVersion": "2099-01-01" });
    let line = json!({ "jsonrpc": "2.0", "id": 7, "method": "tools/list", "params": p });
    assert_eq!(
        handle(&line.to_string(), &mac).unwrap()["error"]["code"],
        -32022
    );
    assert_eq!(ask(&mac, "initialize", json!({}))["error"]["code"], -32601);

    // A handshake client (Claude Code's) gets its version back if it has a
    // handshake, else the newest that does, and then lists tools.
    let init = json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize",
                       "params": { "protocolVersion": "2025-06-18" } });
    let r = handle(&init.to_string(), &mac).unwrap();
    assert_eq!(r["result"]["protocolVersion"], "2025-06-18");
    assert_eq!(r["result"]["serverInfo"]["name"], "multitop-agent");
    let init = json!({ "jsonrpc": "2.0", "id": 1, "method": "initialize",
                       "params": { "protocolVersion": "2026-07-28" } });
    assert_eq!(
        handle(&init.to_string(), &mac).unwrap()["result"]["protocolVersion"],
        "2025-11-25"
    );

    // A notification earns no reply; a bad line earns -32700.
    let note = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
    assert!(handle(&note.to_string(), &mac).is_none());
    assert_eq!(handle("{not json", &mac).unwrap()["error"]["code"], -32700);
}

#[test]
fn serve_answers_line_by_line_until_eof() {
    let mac = Fake::mac();
    let input = format!(
        "{}\n\n{}\n",
        json!({ "jsonrpc": "2.0", "id": 1, "method": "server/discover", "params": { "_meta": meta() } }),
        json!({ "jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": { "_meta": meta() } }),
    );
    let mut out = Vec::new();
    serve(&mut input.as_bytes(), &mut out, &mac).unwrap();
    let replies: Vec<Value> = String::from_utf8(out)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(replies.len(), 2);
    assert_eq!(replies[1]["id"], 2);
}

#[test]
fn mcp_is_a_mode_and_the_usage_names_it() {
    assert_eq!(
        crate::parse_args(["mcp".to_string()]).mode,
        crate::Mode::Mcp
    );
    assert_eq!(crate::Mode::Mcp.word(), "mcp");
    assert!(
        crate::usage().contains("mcp       this machine's cron"),
        "{}",
        crate::usage()
    );
}
