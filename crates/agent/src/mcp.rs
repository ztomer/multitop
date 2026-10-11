//! `multitop-agent mcp`: this machine's ops, over MCP on stdio.
//!
//! The Ops view reads a host's `mcp_host`; a Mac has none, and running the
//! fleet's Linux server there produces false readings (an empty cron table
//! reported as an error, no Docker while Colima runs, a green "no alerts"
//! from a log nothing writes). This server lists only what it can answer
//! here -- `cron.status` from launchd, `docker.containers` from whichever
//! daemon the docker context names -- and says WHY the rest is absent, so the
//! view shows an honest absence instead of a guess.
//!
//! Protocol: 2026-07-28 statelessly (`server/discover`, the version in each
//! request's `_meta`), and the older handshake versions through
//! `initialize`, so Claude Code can register it too. Ported from
//! `routines`' server (`src/mcp_server.rs`), which the fleet's clients were
//! measured against. Line-delimited JSON-RPC; a bad line is answered with
//! -32700 and the session goes on.

use std::io::{BufRead, Write};

use serde_json::{json, Value};

use crate::mcp_tools::{self, System};

/// Versions this server speaks, oldest first.
pub const SUPPORTED_PROTOCOL_VERSIONS: [&str; 5] = [
    "2024-11-05",
    "2025-03-26",
    "2025-06-18",
    "2025-11-25",
    "2026-07-28",
];

/// The first version with no handshake: the version rides in `_meta`.
const STATELESS_SINCE: &str = "2026-07-28";
/// The newest version a handshake can negotiate.
const LATEST_HANDSHAKE_VERSION: &str = "2025-11-25";
/// How long a client may trust `server/discover` and `tools/list`.
/// Reason: what is listed is decided when the session starts (a Docker
/// daemon found or not); a client that reconnects learns any change.
const LIST_TTL_MS: u64 = 3_600_000;
/// What 2026-07-28 removed, for a request that names it.
const REMOVED_IN_2026: [&str; 2] = ["initialize", "ping"];
/// The `_meta` key that names each unlisted section and why it is absent.
/// The Ops view shows the reason; without it all a client can say is that a
/// tool is not listed.
pub const META_ABSENT: &str = "dev.multitop/absent";

fn server_info() -> Value {
    json!({ "name": "multitop-agent", "version": crate::consts::AGENT_VERSION })
}

fn with(mut result: Value, key: &str, value: Value) -> Value {
    if let Some(o) = result.as_object_mut() {
        o.insert(key.to_string(), value);
    }
    result
}

/// Serve on stdin/stdout until EOF.
///
/// # Errors
///
/// As [`serve`].
pub fn run() -> std::io::Result<()> {
    let system = mcp_tools::Live::new();
    serve(
        &mut std::io::stdin().lock(),
        &mut std::io::stdout().lock(),
        &system,
    )
}

/// Serve `input` to `output` until EOF, asking `system` about the machine.
///
/// # Errors
///
/// `input` cannot be read or `output` written. A malformed line is not an
/// error: it is answered and the session continues.
pub fn serve(
    input: &mut dyn BufRead,
    output: &mut dyn Write,
    system: &dyn System,
) -> std::io::Result<()> {
    let mut line = String::new();
    loop {
        line.clear();
        if input.read_line(&mut line)? == 0 {
            return Ok(());
        }
        if line.trim().is_empty() {
            continue;
        }
        if let Some(reply) = handle(line.trim(), system) {
            let mut out = reply.to_string();
            out.push('\n');
            output.write_all(out.as_bytes())?;
            output.flush()?;
        }
    }
}

/// One message's reply; `None` for a notification.
#[must_use]
pub fn handle(line: &str, system: &dyn System) -> Option<Value> {
    let req: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(e) => {
            return Some(json!({
                "jsonrpc": "2.0", "id": null,
                "error": { "code": -32700, "message": format!("parse error: {e}") }
            }));
        }
    };
    let id = req.get("id").cloned()?;
    let method = req
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let error = |code: i64, message: String, data: Option<Value>| {
        let mut e = json!({ "code": code, "message": message });
        if let Some(d) = data {
            e["data"] = d;
        }
        Some(json!({ "jsonrpc": "2.0", "id": id, "error": e }))
    };
    let stateless = req
        .pointer("/params/_meta/io.modelcontextprotocol~1protocolVersion")
        .and_then(Value::as_str)
        .filter(|v| *v >= STATELESS_SINCE);
    if let Some(v) = stateless {
        if !SUPPORTED_PROTOCOL_VERSIONS.contains(&v) {
            return error(
                -32022,
                format!("unsupported protocol version {v}"),
                Some(json!({ "supported": SUPPORTED_PROTOCOL_VERSIONS, "requested": v })),
            );
        }
        if REMOVED_IN_2026.contains(&method) {
            return error(
                -32601,
                format!("method not found: {method} (removed in protocol {v})"),
                None,
            );
        }
    }
    let listed = mcp_tools::listed(system);
    let result = match method {
        "server/discover" => json!({
            "supportedVersions": SUPPORTED_PROTOCOL_VERSIONS,
            "capabilities": { "tools": {} },
            "ttlMs": LIST_TTL_MS,
            "cacheScope": "private",
            "_meta": {
                "io.modelcontextprotocol/serverInfo": server_info(),
                META_ABSENT: mcp_tools::absent(system),
            }
        }),
        "initialize" => json!({
            "protocolVersion": negotiate(&req),
            "capabilities": { "tools": {} },
            "serverInfo": server_info(),
        }),
        "tools/list" => json!({
            "tools": listed.iter().map(|t| t.describe()).collect::<Vec<_>>(),
            "ttlMs": LIST_TTL_MS,
            "cacheScope": "private",
        }),
        "tools/call" => call(&req, &listed, system),
        "ping" => json!({}),
        _ => return error(-32601, format!("method not found: {method}"), None),
    };
    let mut result = with(result, "resultType", json!("complete"));
    if stateless.is_some() {
        // Merged, not replaced: discover's own `_meta` carries the absences.
        let mut meta = result.get("_meta").cloned().unwrap_or_else(|| json!({}));
        meta["io.modelcontextprotocol/serverInfo"] = server_info();
        result = with(result, "_meta", meta);
    }
    Some(json!({ "jsonrpc": "2.0", "id": id, "result": result }))
}

/// `initialize`'s version: the client's when we speak it AND it has a
/// handshake, else the newest that has one.
fn negotiate(req: &Value) -> &'static str {
    let asked = req
        .pointer("/params/protocolVersion")
        .and_then(Value::as_str)
        .unwrap_or_default();
    SUPPORTED_PROTOCOL_VERSIONS
        .into_iter()
        .find(|v| *v == asked && *v < STATELESS_SINCE)
        .unwrap_or(LATEST_HANDSHAKE_VERSION)
}

/// `tools/call`. A tool that fails is a result with `isError` (the MCP
/// convention), never a protocol error; a tool this session did not list is
/// one too -- calling it would answer a question the listing said this host
/// cannot.
fn call(req: &Value, listed: &[mcp_tools::Tool], system: &dyn System) -> Value {
    let Some(name) = req.pointer("/params/name").and_then(Value::as_str) else {
        return failed("tools/call needs a `name`");
    };
    let Some(tool) = listed.iter().find(|t| t.name() == name) else {
        return failed(&format!("no such tool on this host: {name}"));
    };
    match tool.run(system) {
        Ok(v) => json!({
            "content": [{ "type": "text", "text": v.to_string() }],
            "structuredContent": v,
            "isError": false,
        }),
        Err(why) => failed(&why),
    }
}

fn failed(text: &str) -> Value {
    json!({ "content": [{ "type": "text", "text": text }], "isError": true })
}

#[cfg(test)]
#[path = "mcp_tests.rs"]
mod tests;
