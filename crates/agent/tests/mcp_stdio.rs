//! `multitop-agent mcp` over its real stdio: the wire, pinned against the
//! binary rather than against the functions behind it.
//!
//! Written against the hand-rolled server (0.66.0) before it moved onto
//! rmcp, run green there, then run against the move: every difference the
//! move made is either a bug or a delta named in the commit that changed the
//! assertion. Nothing here calls a tool that changes the machine: the two
//! tools read launchd and the Docker daemon, and only `cron.status` is called.

// A test crate, said where clippy reads it (see exec_run_test.rs).
#![cfg(test)]

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{channel, Receiver};
use std::time::Duration;

use serde_json::{json, Value};

const WAIT: Duration = Duration::from_secs(30);
const NEWEST: &str = "2026-07-28";

/// The server, killed and reaped however the test ends.
struct Server {
    child: Child,
    stdin: Option<ChildStdin>,
    lines: Receiver<String>,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Server {
    fn start() -> Result<Self, String> {
        let mut child = Command::new(env!("CARGO_BIN_EXE_multitop-agent"))
            .arg("mcp")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("spawn: {e}"))?;
        let stdin = child.stdin.take();
        let stdout = child.stdout.take().ok_or("no stdout")?;
        let (tx, lines) = channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if tx.send(line).is_err() {
                    return;
                }
            }
        });
        Ok(Self {
            child,
            stdin,
            lines,
        })
    }

    fn send(&mut self, msg: &Value) -> Result<(), String> {
        let stdin = self.stdin.as_mut().ok_or("stdin closed")?;
        writeln!(stdin, "{msg}").map_err(|e| format!("write: {e}"))?;
        stdin.flush().map_err(|e| format!("flush: {e}"))
    }

    fn recv(&self) -> Result<Value, String> {
        let line = self
            .lines
            .recv_timeout(WAIT)
            .map_err(|e| format!("no reply: {e}"))?;
        serde_json::from_str(&line).map_err(|e| format!("not JSON ({e}): {line}"))
    }

    /// One request, its reply.
    fn ask(&mut self, id: u64, method: &str, params: Value) -> Result<Value, String> {
        let mut msg = json!({ "jsonrpc": "2.0", "id": id, "method": method });
        msg["params"] = params;
        self.send(&msg)?;
        let reply = self.recv()?;
        if reply["id"] != json!(id) {
            return Err(format!("reply to another id: {reply}"));
        }
        Ok(reply)
    }

    /// A stateless (2026-07-28) request: the version and the client rides in `_meta`.
    fn ask_stateless(&mut self, id: u64, method: &str, mut params: Value) -> Result<Value, String> {
        params["_meta"] = json!({
            "io.modelcontextprotocol/protocolVersion": NEWEST,
            "io.modelcontextprotocol/clientCapabilities": {},
            "io.modelcontextprotocol/clientInfo": { "name": "mcp_stdio", "version": "1" },
        });
        self.ask(id, method, params)
    }

    /// A handshake session, at `version`.
    fn initialized(version: &str) -> Result<(Self, Value), String> {
        let mut s = Self::start()?;
        let init = s.ask(
            1,
            "initialize",
            json!({
                "protocolVersion": version,
                "capabilities": {},
                "clientInfo": { "name": "mcp_stdio", "version": "1" },
            }),
        )?;
        s.send(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }))?;
        Ok((s, init))
    }
}

fn names(list: &Value) -> Vec<String> {
    list["result"]["tools"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|t| t["name"].as_str().map(str::to_string))
        .collect()
}

#[test]
fn discover_names_the_agent_the_newest_version_and_why_each_section_is_absent() {
    let mut s = Server::start().unwrap();
    let d = s.ask_stateless(1, "server/discover", json!({})).unwrap();
    let r = &d["result"];
    let versions: Vec<&str> = r["supportedVersions"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert!(versions.contains(&NEWEST), "{d}");
    assert!(versions.contains(&"2025-11-25"), "{d}");
    assert_eq!(r["capabilities"]["tools"], json!({}), "{d}");
    assert_eq!(r["ttlMs"], json!(3_600_000), "{d}");
    assert_eq!(r["cacheScope"], json!("private"), "{d}");
    assert_eq!(r["resultType"], json!("complete"), "{d}");
    let info = &r["_meta"]["io.modelcontextprotocol/serverInfo"];
    assert_eq!(info["name"], json!("multitop-agent"), "{d}");
    assert_eq!(info["version"], json!(env!("CARGO_PKG_VERSION")), "{d}");
    let absent = &r["_meta"]["dev.multitop/absent"];
    for tool in ["health.list", "alerts.since", "zfs.health", "host.facts"] {
        assert!(
            absent[tool].as_str().is_some_and(|w| !w.is_empty()),
            "{tool}: {d}"
        );
    }
}

#[test]
fn the_tool_list_is_what_this_machine_can_answer_each_read_only_with_a_schema() {
    let mut s = Server::start().unwrap();
    let list = s.ask_stateless(1, "tools/list", json!({})).unwrap();
    let got = names(&list);
    for n in &got {
        assert!(
            ["cron.status", "docker.containers"].contains(&n.as_str()),
            "{list}"
        );
    }
    if cfg!(target_os = "macos") {
        assert!(got.contains(&"cron.status".to_string()), "{list}");
    }
    assert_eq!(list["result"]["ttlMs"], json!(3_600_000), "{list}");
    for t in list["result"]["tools"].as_array().unwrap() {
        assert_eq!(t["inputSchema"]["type"], json!("object"), "{t}");
        assert_eq!(t["annotations"]["readOnlyHint"], json!(true), "{t}");
        assert!(
            t["description"].as_str().is_some_and(|d| !d.is_empty()),
            "{t}"
        );
    }
}

#[test]
fn initialize_echoes_a_handshake_version_and_names_the_newest_handshake_otherwise() {
    let (_s, init) = Server::initialized("2025-06-18").unwrap();
    assert_eq!(
        init["result"]["protocolVersion"],
        json!("2025-06-18"),
        "{init}"
    );
    assert_eq!(
        init["result"]["serverInfo"]["name"],
        json!("multitop-agent"),
        "{init}"
    );
    assert_eq!(
        init["result"]["serverInfo"]["version"],
        json!(env!("CARGO_PKG_VERSION")),
        "{init}"
    );
    assert!(
        init["result"]["capabilities"]["tools"].is_object(),
        "{init}"
    );
    let (_s, init) = Server::initialized(NEWEST).unwrap();
    assert_eq!(
        init["result"]["protocolVersion"],
        json!("2025-11-25"),
        "{init}"
    );
}

#[test]
fn a_handshake_session_pings_lists_and_ignores_notifications() {
    let (mut s, _) = Server::initialized("2025-06-18").unwrap();
    // The `initialized` notification got no reply: the next line is ping's.
    let ping = s.ask(2, "ping", json!({})).unwrap();
    assert!(ping["result"].is_object(), "{ping}");
    let list = s.ask(3, "tools/list", json!({})).unwrap();
    assert!(list["result"]["tools"].is_array(), "{list}");
}

#[test]
fn cron_status_answers_structured_content_and_the_same_object_as_text() {
    if !cfg!(target_os = "macos") {
        return;
    }
    let mut s = Server::start().unwrap();
    let r = s
        .ask_stateless(
            1,
            "tools/call",
            json!({ "name": "cron.status", "arguments": {} }),
        )
        .unwrap();
    let res = &r["result"];
    assert_ne!(res["isError"], json!(true), "{r}");
    assert_eq!(res["structuredContent"]["source"], json!("launchd"), "{r}");
    assert!(res["structuredContent"]["jobs"].is_array(), "{r}");
    let text: Value = serde_json::from_str(res["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(text, res["structuredContent"], "{r}");
}

#[test]
fn an_unlisted_tool_is_a_tool_error_not_a_protocol_error() {
    let mut s = Server::start().unwrap();
    let r = s
        .ask_stateless(
            1,
            "tools/call",
            json!({ "name": "deploy.run", "arguments": {} }),
        )
        .unwrap();
    assert!(r.get("error").is_none(), "{r}");
    assert_eq!(r["result"]["isError"], json!(true), "{r}");
    let text = r["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("deploy.run"), "{r}");
}

#[test]
fn an_unknown_method_is_method_not_found() {
    let mut s = Server::start().unwrap();
    let r = s.ask_stateless(1, "nope/never", json!({})).unwrap();
    assert_eq!(r["error"]["code"], json!(-32601), "{r}");
}

#[test]
fn end_of_input_ends_the_server_cleanly() {
    let mut s = Server::start().unwrap();
    s.ask_stateless(1, "tools/list", json!({})).unwrap();
    drop(s.stdin.take());
    let deadline = std::time::Instant::now() + WAIT;
    loop {
        if let Some(status) = s.child.try_wait().unwrap() {
            assert!(status.success(), "{status}");
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "still running after EOF"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn a_version_it_does_not_speak_is_refused_with_the_ones_it_does() {
    let mut s = Server::start().unwrap();
    let r = s
        .ask(
            1,
            "tools/list",
            json!({ "_meta": {
                "io.modelcontextprotocol/protocolVersion": "2099-01-01",
                "io.modelcontextprotocol/clientCapabilities": {},
            } }),
        )
        .unwrap();
    assert_eq!(r["error"]["code"], json!(-32022), "{r}");
    assert!(r["error"]["data"]["supported"]
        .as_array()
        .unwrap()
        .contains(&json!(NEWEST)));
}

#[test]
fn a_stateless_request_cannot_reach_what_2026_removed() {
    let mut s = Server::start().unwrap();
    for (id, method) in [(1, "initialize"), (2, "ping")] {
        let r = s.ask_stateless(id, method, json!({})).unwrap();
        assert_eq!(r["error"]["code"], json!(-32601), "{method}: {r}");
    }
}

/// Deliberate delta of the move to rmcp (0.67.0): a line that is not JSON
/// is dropped, where the hand-rolled loop answered -32700. The session goes
/// on either way, which is the property that matters.
#[test]
fn a_line_that_is_not_json_does_not_end_the_session() {
    let mut s = Server::start().unwrap();
    s.stdin
        .as_mut()
        .map(|i| writeln!(i, "{{not json"))
        .unwrap()
        .unwrap();
    let r = s.ask_stateless(2, "tools/list", json!({})).unwrap();
    assert!(r["result"]["tools"].is_array(), "{r}");
}

/// The rollout: a multitop older than 0.67.0 sends its stateless `_meta`
/// with the version and its capabilities but no `clientInfo`. Its requests
/// must still be answered, or an older client loses the local host's card.
#[test]
fn an_older_multitops_requests_without_client_info_are_answered() {
    let mut s = Server::start().unwrap();
    let meta = json!({
        "io.modelcontextprotocol/protocolVersion": NEWEST,
        "io.modelcontextprotocol/clientCapabilities": {},
    });
    let d = s
        .ask(1, "server/discover", json!({ "_meta": meta }))
        .unwrap();
    assert!(d["result"]["supportedVersions"].is_array(), "{d}");
    let list = s.ask(2, "tools/list", json!({ "_meta": meta })).unwrap();
    assert!(list["result"]["tools"].is_array(), "{list}");
}
