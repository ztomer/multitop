//! An in-memory `mcp_host` for tests: answers each request from a table,
//! records what it was sent, and can go quiet or hang up on cue.

use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, DuplexStream};

/// How to answer one method.
#[derive(Clone)]
pub enum Reply {
    Result(Value),
    Error(i64, &'static str),
    /// Never answer (a server gone quiet).
    Silent,
    /// Close the pipe instead of answering.
    HangUp,
}

/// The requests the fake saw, in order.
pub type Seen = Arc<Mutex<Vec<Value>>>;

/// A `server/discover` answer offering `versions`, naming `mcp_host 1.0.0`.
#[must_use]
pub fn discover(versions: &[&str]) -> Value {
    json!({
        "supportedVersions": versions,
        "capabilities": {},
        "_meta": {"io.modelcontextprotocol/serverInfo": {"name": "mcp_host", "version": "1.0.0"}},
    })
}

/// A `tools/list` page naming `tools`.
#[must_use]
pub fn tools(names: &[&str]) -> Value {
    json!({ "tools": names.iter().map(|n| json!({"name": n})).collect::<Vec<_>>() })
}

/// A client-side pipe pair wired to a fake answering `route(method, params)`.
pub fn serve(
    route: impl Fn(&str, &Value) -> Reply + Send + 'static,
) -> (
    tokio::io::ReadHalf<DuplexStream>,
    tokio::io::WriteHalf<DuplexStream>,
    Seen,
) {
    let (client, server) = tokio::io::duplex(1 << 16);
    let (sr, mut sw) = tokio::io::split(server);
    let seen: Seen = Arc::default();
    let log = Arc::clone(&seen);
    tokio::spawn(async move {
        let mut lines = BufReader::new(sr).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let req: Value = serde_json::from_str(&line).unwrap_or_default();
            if let Ok(mut l) = log.lock() {
                l.push(req.clone());
            }
            let method = req["method"].as_str().unwrap_or_default().to_string();
            let reply = match route(&method, &req["params"]) {
                Reply::Result(r) => json!({"jsonrpc": "2.0", "id": req["id"], "result": r}),
                Reply::Error(code, message) => {
                    json!({"jsonrpc": "2.0", "id": req["id"], "error": {"code": code, "message": message}})
                }
                Reply::Silent => continue,
                Reply::HangUp => return,
            };
            // A notification first, as a real server may interleave one.
            let note = json!({"jsonrpc": "2.0", "method": "notifications/message", "params": {}});
            let out = format!("{note}\n{reply}\n");
            if sw.write_all(out.as_bytes()).await.is_err() {
                return;
            }
        }
    });
    let (cr, cw) = tokio::io::split(client);
    (cr, cw, seen)
}
