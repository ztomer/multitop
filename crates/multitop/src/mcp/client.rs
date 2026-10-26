//! A stateless MCP client (protocol 2026-07-28) for a host's `mcp_host`,
//! on rmcp, the official Rust SDK (owner ruling OW.60, queue MCP.MON).
//!
//! The protocol is rmcp's: one JSON-RPC message per line on the server's
//! stdin and stdout, no handshake (`server/discover` first, every request
//! after it naming the version in its `_meta`), and the id matching that
//! keeps a late answer from being taken for the next one. What stays here
//! is this client's own reading of the estate: the absent map and the
//! catalogue digest in discover's `_meta`, the kept tool list, and the
//! deadline per answer. A server that does not offer
//! 2026-07-28 is refused with the versions it does offer, rather than
//! spoken to in a dialect this client was never tested against. Each
//! request has a deadline; a server that goes quiet is a stated timeout,
//! one that closes its pipe a stated close - never a hang.
//!
//! Read-only by construction: this client declares no capabilities, so the
//! server never asks it to confirm anything, and the Ops view only calls
//! tools that read.
//!
//! **A MISSED DEADLINE IS A GAP IN ONE ANSWER.** A server that goes quiet
//! is a stated timeout, naming the CALL that missed its deadline, and the
//! session is still good: the estate's other readings are read (servers
//! ROADMAP E74). Only a closed pipe ends the session.

use std::future::Future;
use std::marker::PhantomData;
use std::time::Duration;

use rmcp::model::{
    CallToolRequest, CallToolRequestParams, ClientCapabilities, ClientConfig, ClientRequest,
    Implementation, ListToolsRequest, PaginatedRequestParams, ProtocolVersion, ReadResourceRequest,
    ReadResourceRequestParams, ServerResult,
};
use rmcp::service::{ClientLifecycleMode, ClientServiceExt, RunningService};
use rmcp::{RoleClient, ServiceError};
use serde_json::Value;
use tokio::io::{AsyncRead, AsyncWrite};

use super::catalogue;
pub use super::error::Error;

/// The one version this client speaks.
pub const PROTOCOL_VERSION: &str = "2026-07-28";
/// The request `_meta` key naming the version (2026-07-28 `RequestMeta`).
pub const META_PROTOCOL_VERSION: &str = "io.modelcontextprotocol/protocolVersion";
/// The request `_meta` key carrying this client's capabilities.
pub const META_CLIENT_CAPABILITIES: &str = "io.modelcontextprotocol/clientCapabilities";

/// The tool list the server gave, and the window it said it may be kept.
struct Listing {
    /// Every tool the server listed, every page of them.
    tools: Vec<String>,
    /// How long the server said the list may be kept (`ttlMs`).
    ttl_ms: u64,
}

/// One session with one server.
pub struct Client<R, W> {
    service: RunningService<RoleClient, ClientConfig>,
    /// The pipe halves this session was opened over; they now belong to
    /// rmcp's transport task.
    io: PhantomData<fn() -> (R, W)>,
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
    /// Set by the first failure that ends the session (a closed pipe - the
    /// server exited, or ssh did).
    ///
    /// **A MISSED DEADLINE DOES NOT END THE SESSION.** The old reason was
    /// that "a late reply could be mistaken for the next one's" - and it
    /// cannot: a reply carries the id of the request it answers, and this
    /// client matches that id, so a late answer is read and discarded,
    /// never taken for the next one (servers ROADMAP E74). A session that
    /// ends on the first missed deadline throws away the estate's OTHER
    /// readings with it - and the local host lost its whole card to a
    /// `docker.containers` call that missed the deadline on 17 of about
    /// 124 passes of the poll's log.
    broken: Option<Error>,
}

impl<R, W> Client<R, W>
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
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
            service: open(reader, writer, timeout).await?,
            io: PhantomData,
            timeout,
            server: (String::new(), String::new()),
            tools: Vec::new(),
            absent: std::collections::HashMap::new(),
            catalogue: catalogue::Catalogue::Listed { ttl_s: 0 },
            broken: None,
        };
        // The discover answer, as the server sent it: rmcp keeps its
        // `_meta` beside the version it selected.
        let d = c
            .service
            .peer()
            .peer_info()
            .and_then(|i| serde_json::to_value(&*i).ok())
            .unwrap_or_default();
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
    /// Asked as raw requests rather than through rmcp's `list_all_tools`:
    /// that one answers from rmcp's own response cache, and the list is
    /// kept HERE, against the server's digest (servers ROADMAP E72).
    ///
    /// The pages are followed and the tools are accumulated: a list that
    /// kept only the first page would lose every tool on the second, and a
    /// tool that is not in the list has no section in the panel.
    async fn list_tools(&mut self) -> Result<Listing, Error> {
        let mut cursor: Option<String> = None;
        let mut tools: Vec<String> = Vec::new();
        let mut ttl_ms = 0;
        loop {
            let mut params = PaginatedRequestParams::default();
            params.cursor = cursor;
            let request = ClientRequest::ListToolsRequest(ListToolsRequest::with_param(params));
            let page = self.request(request, self.timeout, "tools/list").await?;
            if ttl_ms == 0 {
                ttl_ms = page.get("ttlMs").and_then(Value::as_u64).unwrap_or(0);
            }
            // An answer rmcp could not read as a tool list is NOT an empty
            // list: a host whose list is unreadable would otherwise draw as a
            // host with nothing to show.
            let listed = page.get("tools").and_then(Value::as_array).ok_or_else(|| {
                Error::Protocol("tools/list: the answer is not a tool list".to_string())
            })?;
            tools.extend(
                listed
                    .iter()
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
}

impl<R, W> Client<R, W> {
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

    /// How long each request may take: the estate's deadline for one
    /// answer.
    ///
    /// The estate's, not this client's: it is the budget the caller was
    /// given, and it is not raised to make a failure go away (servers
    /// ROADMAP E74).
    #[must_use]
    pub const fn timeout(&self) -> Duration {
        self.timeout
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
    /// check's own probe, about half a minute - so the tool that needs
    /// the room has to ask for it here rather than have it cut short.
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
        let mut params = CallToolRequestParams::new(tool.to_string());
        if let Value::Object(arguments) = args {
            params = params.with_arguments(arguments);
        }
        let request = ClientRequest::CallToolRequest(CallToolRequest::new(params));
        let r = self.request(request, timeout, tool).await?;
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
        let request = ClientRequest::ReadResourceRequest(ReadResourceRequest::new(
            ReadResourceRequestParams::new(uri),
        ));
        let r = self.request(request, self.timeout, uri).await?;
        let text = r
            .pointer("/contents/0/text")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::Protocol(format!("{uri}: no text content")))?;
        serde_json::from_str(text).map_err(|e| Error::Protocol(format!("{uri}: {e}")))
    }

    /// One request with a deadline of its own, and a NAME for what was
    /// asked; its answer as the JSON the server sent.
    ///
    /// The name is what the operator reads when the deadline is missed:
    /// `docker.containers: no answer in 20s`, not `tools/call: no answer
    /// in 20s` - which named a method and no tool at all, so a log full
    /// of timeouts said nothing about which call had missed its deadline
    /// (servers ROADMAP E74).
    ///
    /// **A MISSED DEADLINE IS A GAP IN ONE ANSWER, NOT THE END OF THE
    /// SESSION.** The session ends when the pipe closes - the estate's
    /// session is gone - and not when one answer is late: rmcp matches each
    /// answer to its request's id, so a late answer is discarded rather than
    /// taken for the next one, and the estate's other readings are still
    /// read, which is the whole of E74.
    ///
    /// Every request carries the 2026-07-28 `_meta` (the version, this
    /// client's empty capabilities and its identity): rmcp adds it to each
    /// request once discover has selected the version.
    ///
    /// # Errors
    ///
    /// [`Error::Closed`], [`Error::Timeout`], [`Error::Rpc`] or
    /// [`Error::Protocol`].
    async fn request(
        &mut self,
        request: ClientRequest,
        timeout: Duration,
        what: &str,
    ) -> Result<Value, Error> {
        if let Some(e) = &self.broken {
            return Err(e.clone());
        }
        let out = within(timeout, what, self.service.peer().send_request(request)).await;
        if let Err(e @ Error::Closed(_)) = &out {
            self.broken = Some(e.clone());
        }
        out
    }
}

/// `fut`'s answer as JSON, or why there is none, within `timeout`.
async fn within(
    timeout: Duration,
    what: &str,
    fut: impl Future<Output = Result<ServerResult, ServiceError>>,
) -> Result<Value, Error> {
    let answer = tokio::time::timeout(timeout, fut)
        .await
        .map_err(|_| Error::Timeout {
            what: what.to_string(),
            after: timeout,
        })?
        .map_err(|e| Error::of_service(e, what))?;
    serde_json::to_value(answer).map_err(|e| Error::Protocol(format!("{what}: {e}")))
}

/// Open the session: `server/discover` at 2026-07-28 and nothing older,
/// within `timeout`.
async fn open<R, W>(
    reader: R,
    writer: W,
    timeout: Duration,
) -> Result<RunningService<RoleClient, ClientConfig>, Error>
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let mut info = ClientConfig::default();
    info.capabilities = ClientCapabilities::default();
    info.client_info = Implementation::new("multitop", env!("CARGO_PKG_VERSION"));
    let lifecycle = ClientLifecycleMode::Discover {
        preferred_versions: vec![ProtocolVersion::V_2026_07_28],
    };
    tokio::time::timeout(
        timeout,
        info.serve_with_lifecycle((reader, writer), lifecycle),
    )
    .await
    .map_err(|_| Error::Timeout {
        what: "server/discover".to_string(),
        after: timeout,
    })?
    .map_err(Error::of_opening)
}
