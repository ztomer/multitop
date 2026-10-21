//! A stateless MCP client (protocol 2026-07-28) for a host's `mcp_host`.
//!
//! One JSON-RPC request per line on the server's stdin, one reply per line
//! on its stdout, no handshake: `server/discover` first, and every request
//! after it names the version in its `_meta`. A server that does not offer
//! 2026-07-28 is refused with the versions it does offer, rather than
//! spoken to in a dialect this client was never tested against. Each
//! request has a deadline; a server that goes quiet is a stated timeout,
//! one that closes its pipe a stated close - never a hang.
//!
//! Read-only by construction: this client declares no capabilities, so the
//! server never asks it to confirm anything, and the Ops view only calls
//! tools that read.

use std::fmt;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader, Lines};

use super::catalogue;

/// The one version this client speaks.
pub const PROTOCOL_VERSION: &str = "2026-07-28";
/// The request `_meta` key naming the version (2026-07-28 `RequestMeta`).
pub const META_PROTOCOL_VERSION: &str = "io.modelcontextprotocol/protocolVersion";
/// The request `_meta` key carrying this client's capabilities.
pub const META_CLIENT_CAPABILITIES: &str = "io.modelcontextprotocol/clientCapabilities";

/// Why a request did not produce an answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The pipe closed or broke: the server exited, or ssh did.
    Closed(String),
    /// No reply within the deadline.
    Timeout { method: String, after: Duration },
    /// A JSON-RPC error reply.
    Rpc { code: i64, message: String },
    /// The tool ran and reported failure (`isError`), with its text.
    Tool(String),
    /// The server does not offer 2026-07-28; these are what it offers.
    Unsupported(Vec<String>),
    /// A reply this client cannot read.
    Protocol(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Closed(why) => write!(f, "mcp_host closed the session: {why}"),
            Self::Timeout { method, after } if after.as_secs() == 0 => {
                write!(f, "{method}: no answer in {}ms", after.as_millis())
            }
            Self::Timeout { method, after } => {
                write!(f, "{method}: no answer in {}s", after.as_secs())
            }
            Self::Rpc { code, message } => write!(f, "{message} ({code})"),
            Self::Tool(text) => write!(f, "{text}"),
            Self::Unsupported(v) => write!(
                f,
                "mcp_host offers protocol {} - this client speaks {PROTOCOL_VERSION} only",
                if v.is_empty() {
                    "none".to_string()
                } else {
                    v.join(", ")
                }
            ),
            Self::Protocol(why) => write!(f, "unreadable answer: {why}"),
        }
    }
}

impl std::error::Error for Error {}

/// The tool list the server gave, and the window it said it may be kept.
struct Listing {
    /// Every tool the server listed, every page of them.
    tools: Vec<String>,
    /// How long the server said the list may be kept (`ttlMs`).
    ttl_ms: u64,
}

/// One session with one server.
pub struct Client<R, W> {
    lines: Lines<BufReader<R>>,
    writer: W,
    next_id: u64,
    timeout: Duration,
    /// `serverInfo.name` and `.version`, from `server/discover`.
    pub server: (String, String),
    /// Every tool the server lists, by name.
    pub tools: Vec<String>,
    /// Where the tool list came from: the server, or the kept list.
    ///
    /// The list is the panel's SHAPE - a tool the list does not name has no
    /// section - so where it came from, and how old it is, travels with the
    /// answer the panel draws. A list nobody can date is a list nobody can
    /// trust (servers ROADMAP E61).
    pub catalogue: catalogue::Catalogue,
    /// Why the server does not list a tool the Ops view reads, by tool name,
    /// when it says. Both servers say: `multitop-agent mcp` publishes the
    /// tools it cannot answer, and so does the estate's `mcp_host` (its
    /// `dev.multitop/absent` meta names `zfs.health` and the rest it
    /// withheld, with the reason each was withheld). This comment used to
    /// say `mcp_host` does not -- measured against `.158` on 2026-10-19, it
    /// does, and the Ops view reads its reasons.
    pub absent: std::collections::HashMap<String, String>,
    /// Set by the first failure that ends the session (a closed pipe or a
    /// deadline missed - after a timeout a late reply could be mistaken
    /// for the next one's, so the session is not trusted again).
    broken: Option<Error>,
}

impl<R: AsyncRead + Unpin, W: AsyncWrite + Unpin> Client<R, W> {
    /// Discover the server and list its tools; every request, these two
    /// included, has `timeout` to answer.
    ///
    /// The list is asked for every time here: this is the client with no
    /// catalogue kept, and it is the shape the tests and the one-shot paths
    /// use. [`Client::connect_with`] is the client that keeps one.
    ///
    /// # Errors
    ///
    /// The server could not be reached, does not offer 2026-07-28, or
    /// answered something unreadable.
    pub async fn connect(reader: R, writer: W, timeout: Duration) -> Result<Self, Error> {
        Self::connect_with(reader, writer, timeout, &catalogue::Store::nowhere()).await
    }

    /// Discover the server and list its tools, taking the list from `store`
    /// when the kept list IS the list this server is offering now.
    ///
    /// servers ROADMAP E61: the list is the biggest thing this client pulls
    /// off a host, and it changes at a RELEASE - so it is kept between
    /// sessions.
    ///
    /// servers ROADMAP E72: the kept list is checked against the SERVER'S
    /// OWN WORD - the digest of the exact list `tools/list` answers, which
    /// the server publishes in `server/discover` beside the absent map. A
    /// digest that matches keeps the list, even across a release that
    /// touched no tool table (four of the last ten releases did not, and
    /// each of those cost the estate a whole catalogue for nothing); a
    /// digest that does not match puts the kept list aside and the host is
    /// asked again - which is how a table that moved WITHOUT a release is
    /// caught at last.
    ///
    /// A server that publishes NO digest has said nothing about its list:
    /// the kept list is then checked the older way (the version and the
    /// absent set), and the answer says the digest was ABSENT, because an
    /// absent fact is not an all-clear. The client does not re-list every
    /// session in that case - that is the defect the digest closes, not a
    /// new one.
    ///
    /// # Errors
    ///
    /// As [`Client::connect`].
    pub async fn connect_with(
        reader: R,
        writer: W,
        timeout: Duration,
        store: &catalogue::Store,
    ) -> Result<Self, Error> {
        let mut c = Self {
            lines: BufReader::new(reader).lines(),
            writer,
            next_id: 0,
            timeout,
            server: (String::new(), String::new()),
            tools: Vec::new(),
            absent: std::collections::HashMap::new(),
            catalogue: catalogue::Catalogue::Listed { ttl_s: 0 },
            broken: None,
        };
        let d = c.request("server/discover", json!({})).await?;
        let offered: Vec<String> = d
            .get("supportedVersions")
            .and_then(Value::as_array)
            .map(|v| {
                v.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        if !offered.iter().any(|v| v == PROTOCOL_VERSION) {
            return Err(Error::Unsupported(offered));
        }
        let info = |k: &str| {
            d.pointer(&format!("/_meta/io.modelcontextprotocol~1serverInfo/{k}"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string()
        };
        c.server = (info("name"), info("version"));
        if let Some(why) = d
            .pointer(&format!(
                "/_meta/{}",
                multitop_agent::mcp::META_ABSENT.replace('/', "~1")
            ))
            .and_then(Value::as_object)
        {
            c.absent = why
                .iter()
                .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_string())))
                .collect();
        }
        // The server's OWN WORD about the list it serves: the digest of the
        // exact list `tools/list` answers, published beside the absent map
        // (servers ROADMAP E72). A server that publishes none - one too old
        // to know the field, or one publishing something that is not a
        // digest - has said nothing about its list, and the client says so
        // rather than drawing the absence as an all-clear.
        let digest = d
            .pointer(&format!(
                "/_meta/{}",
                catalogue::META_DIGEST.replace('/', "~1")
            ))
            .and_then(Value::as_str)
            .filter(|s| catalogue::is_digest(s))
            .map(str::to_string);
        // The tool list: the KEPT one when it is the list this server is
        // offering now, the server's own list otherwise. The check is made
        // against the LIVE server's own word - the digest when it published
        // one, the older key (the version and the absent set) when it did
        // not - so a release that touched no tool table no longer costs a
        // download, and a table that moved without a release is caught.
        let version = c.server.1.clone();
        let absent = catalogue::fingerprint(&c.absent);
        match store.hit(&version, &absent, digest.as_deref()) {
            catalogue::Hit::Current {
                tools,
                age_s,
                ttl_s,
                proof,
            } => {
                c.tools = tools;
                c.catalogue = catalogue::Catalogue::Kept {
                    age_s,
                    ttl_s,
                    proof,
                };
            }
            catalogue::Hit::Missed(miss) => {
                let listing = c.list_tools().await?;
                let ttl_s = i64::try_from(listing.ttl_ms / catalogue::MS_PER_S).unwrap_or(i64::MAX);
                if let Err(why) = store.store(
                    &version,
                    &absent,
                    digest.as_deref(),
                    &listing.tools,
                    listing.ttl_ms,
                ) {
                    crate::diag::report(&format!("the tool list was not kept: {why}"));
                }
                c.tools = listing.tools;
                c.catalogue = match miss {
                    catalogue::Miss::None => catalogue::Catalogue::Listed { ttl_s },
                    other => catalogue::Catalogue::Relisted { miss: other, ttl_s },
                };
            }
        }
        Ok(c)
    }

    /// `tools/list`, every page of it, and the window the server gave.
    ///
    /// The pages are followed and the tools are accumulated: a list that
    /// kept only the first page would lose every tool on the second, and a
    /// tool that is not in the list has no section in the panel.
    async fn list_tools(&mut self) -> Result<Listing, Error> {
        let mut cursor: Option<String> = None;
        let mut tools: Vec<String> = Vec::new();
        let mut ttl_ms = 0;
        loop {
            let params = cursor.map_or_else(|| json!({}), |c| json!({ "cursor": c }));
            let page = self.request("tools/list", params).await?;
            if ttl_ms == 0 {
                ttl_ms = page.get("ttlMs").and_then(Value::as_u64).unwrap_or(0);
            }
            tools.extend(
                page.get("tools")
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter_map(|t| t.get("name").and_then(Value::as_str))
                    .map(str::to_string),
            );
            cursor = page
                .get("nextCursor")
                .and_then(Value::as_str)
                .map(str::to_string);
            if cursor.is_none() {
                break;
            }
        }
        Ok(Listing { tools, ttl_ms })
    }

    /// Why this session can no longer be used, once it cannot.
    #[must_use]
    pub const fn broken(&self) -> Option<&Error> {
        self.broken.as_ref()
    }

    /// Why `tool` is not listed: the server's own reason, else `fallback`.
    #[must_use]
    pub fn absent_reason(&self, tool: &str, fallback: &str) -> String {
        self.absent
            .get(tool)
            .cloned()
            .unwrap_or_else(|| fallback.to_string())
    }

    /// Whether the server lists `tool`: a host lists only what it can
    /// answer (no docker, no `docker.containers`).
    #[must_use]
    pub fn has_tool(&self, tool: &str) -> bool {
        self.tools.iter().any(|t| t == tool)
    }

    /// `tools/call`: the answer's `structuredContent`.
    ///
    /// # Errors
    ///
    /// As [`Client::request`]; a tool that reports failure is
    /// [`Error::Tool`] with its text, and an answer without structured
    /// content is a protocol error (every fleet tool types its answers).
    pub async fn call(&mut self, tool: &str, args: Value) -> Result<Value, Error> {
        let timeout = self.timeout;
        self.call_within(tool, args, timeout).await
    }

    /// `tools/call` with a deadline of its own.
    ///
    /// Reason: a tool that probes the network needs longer than a file
    /// read - `net.egress` measures the tunnel's exit country with the
    /// check's own probe, about half a minute - and a missed deadline
    /// BREAKS the session, so the tool that needs the room has to ask for
    /// it here rather than have it cut short.
    ///
    /// # Errors
    ///
    /// As [`Client::call`].
    pub async fn call_within(
        &mut self,
        tool: &str,
        args: Value,
        timeout: Duration,
    ) -> Result<Value, Error> {
        let r = self
            .request_within(
                "tools/call",
                json!({ "name": tool, "arguments": args }),
                timeout,
            )
            .await?;
        if r.get("isError").and_then(Value::as_bool) == Some(true) {
            let text = r
                .pointer("/content/0/text")
                .and_then(Value::as_str)
                .unwrap_or("the tool failed and said nothing");
            return Err(Error::Tool(format!("{tool}: {text}")));
        }
        r.get("structuredContent")
            .cloned()
            .ok_or_else(|| Error::Protocol(format!("{tool}: no structuredContent")))
    }

    /// `resources/read` of `uri`: its first content's text, parsed as JSON
    /// (the fleet's resources are `application/json`).
    ///
    /// # Errors
    ///
    /// As [`Client::request`]; a content that is not JSON is a protocol
    /// error.
    pub async fn read(&mut self, uri: &str) -> Result<Value, Error> {
        let r = self
            .request("resources/read", json!({ "uri": uri }))
            .await?;
        let text = r
            .pointer("/contents/0/text")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::Protocol(format!("{uri}: no text content")))?;
        serde_json::from_str(text).map_err(|e| Error::Protocol(format!("{uri}: {e}")))
    }

    /// One request, stateless: its `_meta` names the version and this
    /// client's (empty) capabilities. Notifications and replies to other
    /// ids are skipped.
    ///
    /// # Errors
    ///
    /// [`Error::Closed`], [`Error::Timeout`], [`Error::Rpc`] or
    /// [`Error::Protocol`].
    pub async fn request(&mut self, method: &str, params: Value) -> Result<Value, Error> {
        let timeout = self.timeout;
        self.request_within(method, params, timeout).await
    }

    /// One request with a deadline of its own.
    ///
    /// # Errors
    ///
    /// [`Error::Closed`], [`Error::Timeout`], [`Error::Rpc`] or
    /// [`Error::Protocol`].
    pub async fn request_within(
        &mut self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, Error> {
        if let Some(e) = &self.broken {
            return Err(e.clone());
        }
        let out = self.exchange(method, params, timeout).await;
        if let Err(e @ (Error::Closed(_) | Error::Timeout { .. })) = &out {
            self.broken = Some(e.clone());
        }
        out
    }

    async fn exchange(
        &mut self,
        method: &str,
        mut params: Value,
        timeout: Duration,
    ) -> Result<Value, Error> {
        self.next_id += 1;
        let id = self.next_id;
        params["_meta"] = json!({
            META_PROTOCOL_VERSION: PROTOCOL_VERSION,
            META_CLIENT_CAPABILITIES: {},
        });
        let mut line =
            json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }).to_string();
        line.push('\n');
        self.writer
            .write_all(line.as_bytes())
            .await
            .map_err(|e| Error::Closed(e.to_string()))?;
        self.writer
            .flush()
            .await
            .map_err(|e| Error::Closed(e.to_string()))?;
        let after = timeout;
        let reply = tokio::time::timeout(after, self.reply(id))
            .await
            .map_err(|_| Error::Timeout {
                method: method.to_string(),
                after,
            })??;
        if let Some(e) = reply.get("error") {
            return Err(Error::Rpc {
                code: e.get("code").and_then(Value::as_i64).unwrap_or_default(),
                message: e
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("error without a message")
                    .to_string(),
            });
        }
        reply
            .get("result")
            .cloned()
            .ok_or_else(|| Error::Protocol(format!("{method}: neither result nor error")))
    }

    async fn reply(&mut self, id: u64) -> Result<Value, Error> {
        loop {
            let line = self
                .lines
                .next_line()
                .await
                .map_err(|e| Error::Closed(e.to_string()))?
                .ok_or_else(|| Error::Closed("end of output".to_string()))?;
            if line.trim().is_empty() {
                continue;
            }
            let v: Value = serde_json::from_str(&line)
                .map_err(|e| Error::Protocol(format!("not JSON: {e}")))?;
            if v.get("id").and_then(Value::as_u64) == Some(id) {
                return Ok(v);
            }
        }
    }
}
