//! The server's decisions, over a fake machine: what it lists, what it
//! answers, and why the rest is absent. The protocol itself is rmcp's, and
//! its wire is pinned against the real binary in `tests/mcp_stdio.rs`.

use std::sync::Mutex;

use serde_json::{json, Value};

use super::*;
use crate::docker_transport::DockerEndpoint;

struct Fake {
    launchd: Option<&'static str>,
    labels: Vec<String>,
    docker: Option<DockerEndpoint>,
    containers: Result<Value, String>,
    asked_ages: Mutex<Vec<u32>>,
}

impl Fake {
    fn mac() -> Self {
        Self {
            launchd: Some("PID\tStatus\tLabel\n42\t0\tcom.me.watch\n-\t78\tcom.me.nightly\n-\t0\tcom.apple.x\n"),
            labels: vec!["com.me.nightly".into(), "com.me.watch".into()],
            docker: Some(DockerEndpoint::Unix("/h/.colima/default/docker.sock".into())),
            containers: Ok(json!({ "containers": [{ "name": "web", "state": "running" }] })),
            asked_ages: Mutex::new(Vec::new()),
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
        self.asked_ages.lock().unwrap().extend_from_slice(pids);
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

fn server(system: Fake) -> Server<Fake> {
    Server::new(system)
}

fn names(s: &Server<Fake>) -> Vec<String> {
    s.tools().iter().map(|t| t.name.to_string()).collect()
}

fn text(r: &CallToolResult) -> String {
    let v = serde_json::to_value(r).unwrap();
    v["content"][0]["text"].as_str().unwrap().to_string()
}

#[test]
fn discover_says_why_each_unlisted_section_is_absent() {
    let absent = server(Fake::mac()).absent();
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
    assert_eq!(
        names(&server(Fake::mac())),
        ["cron.status", "docker.containers"]
    );

    let bare = server(Fake {
        launchd: None,
        docker: None,
        ..Fake::mac()
    });
    multitop_testassert::assert_empty!(names(&bare));
    let absent = bare.absent();
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
    let r = bare.call("docker.containers");
    assert_eq!(r.is_error, Some(true), "{r:?}");
}

#[test]
fn every_tool_states_both_hints_and_the_shape_of_its_answer() {
    for t in server(Fake::mac()).tools() {
        let a = t.annotations.as_ref().unwrap();
        assert_eq!(a.read_only_hint, Some(true), "{}", t.name);
        assert_eq!(a.destructive_hint, Some(false), "{}", t.name);
        assert_eq!(a.open_world_hint, Some(false), "{}", t.name);
        let schema = t.output_schema.as_ref().unwrap();
        assert_eq!(schema["type"], "object", "{}", t.name);
    }
}

#[test]
fn a_destructive_tool_is_never_listed() {
    // The table holds none today; this pins that the listing filter is the
    // effect, so one added to the table stays unlisted until an opt-in
    // exists for it.
    for t in mcp_tools::ALL {
        let listed = mcp_tools::listed(&Fake::mac()).contains(&t);
        assert!(
            !(listed && t.effect() == mcp_tools::Effect::Destructive),
            "{t:?}"
        );
    }
}

#[test]
fn each_success_carries_every_field_its_schema_requires() {
    let s = server(Fake::mac());
    for t in mcp_tools::ALL {
        let mut fake = Fake::mac();
        if t == mcp_tools::Tool::Containers {
            fake.containers = Ok(json!({ "endpoint": "unix:///x", "containers": [] }));
        }
        let r = server(fake).call(t.name());
        let out = r.structured_content.unwrap();
        for field in t.output_schema()["required"].as_array().unwrap() {
            assert!(
                out.get(field.as_str().unwrap()).is_some(),
                "{t:?}: {field} in {out}"
            );
        }
    }
    drop(s);
}

#[test]
fn cron_status_is_the_users_agents_with_ages_asked_only_for_them() {
    let s = server(Fake::mac());
    let r = s.call("cron.status");
    assert_eq!(r.is_error, Some(false), "{r:?}");
    let out = r.structured_content.clone().unwrap();
    let jobs = out["jobs"].as_array().unwrap();
    assert_eq!(
        jobs.len(),
        2,
        "Apple's labels are not the user's jobs: {jobs:?}"
    );
    assert_eq!(jobs[0]["label"], "com.me.nightly");
    assert_eq!(jobs[0]["last_rc"], 78);
    assert_eq!(jobs[1]["running_since"], 1_000 - 60);
    assert_eq!(*s.system.asked_ages.lock().unwrap(), [42]);
    // The text content carries the same answer for a client without
    // structured content.
    let as_text: Value = serde_json::from_str(&text(&r)).unwrap();
    assert_eq!(as_text, out);
}

#[test]
fn a_daemon_that_fails_is_a_tool_error_with_its_reason() {
    let down = server(Fake {
        containers: Err("docker at unix:///x: Connection refused".into()),
        ..Fake::mac()
    });
    let r = down.call("docker.containers");
    assert_eq!(r.is_error, Some(true));
    assert!(text(&r).contains("refused"), "{r:?}");
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
