//! `multitop-agent mcp`: this machine's ops, over MCP on stdio, on rmcp.
//!
//! The Ops view reads a host's `mcp_host`; a Mac has none, and running the
//! fleet's Linux server there produces false readings (an empty cron table
//! reported as an error, no Docker while Colima runs, a green "no alerts"
//! from a log nothing writes). This server lists only what it can answer
//! here -- `cron.status` from launchd, `docker.containers` from whichever
//! daemon the docker context names -- and says WHY the rest is absent, so the
//! view shows an honest absence instead of a guess.
//!
//! Until 0.67.0 this file was a hand-rolled JSON-RPC loop with its own
//! version negotiation. The protocol is rmcp's to keep now (owner ruling
//! OW.60, queue MCP.MON): 2026-07-28 statelessly (`server/discover`, the
//! version in each request's `_meta`) and the older versions through
//! `initialize`. What is left here is what only this agent can say -- which
//! tools this machine answers ([`mcp_tools::listed`]), what each does to it
//! ([`mcp_tools::Effect`]), and why the rest are absent ([`META_ABSENT`]).
//!
//! The decisions are plain synchronous methods on [`Server`], tested without
//! a runtime or a pipe; the `ServerHandler` impl is a thin adapter, and
//! [`run`] is the only place tokio appears.

use std::future::Future;
use std::sync::Arc;

use rmcp::model::{
    CacheScope, CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock,
    DiscoverResult, Implementation, JsonObject, ListToolsResult, PaginatedRequestParams,
    ProtocolVersion, ServerCapabilities, ServerConfig, Tool, ToolAnnotations,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData, RoleServer, ServerHandler, ServiceExt};
use serde_json::Value;

use crate::mcp_tools::{self, Effect, System};

/// How long a client may trust `server/discover` and `tools/list`.
/// Reason: what is listed is decided when the session starts (a Docker
/// daemon found or not); a client that reconnects learns any change.
pub const LIST_TTL_MS: u64 = 3_600_000;
/// The `_meta` key that names each unlisted section and why it is absent.
/// The Ops view shows the reason; without it all a client can say is that a
/// tool is not listed.
pub const META_ABSENT: &str = "dev.multitop/absent";
/// The name this server gives itself (`serverInfo.name`).
pub const SERVER_NAME: &str = "multitop-agent";

/// The agent's MCP face over one machine.
pub struct Server<S> {
    system: Arc<S>,
}

impl<S> Clone for Server<S> {
    fn clone(&self) -> Self {
        Self {
            system: Arc::clone(&self.system),
        }
    }
}

impl<S: System> Server<S> {
    pub fn new(system: S) -> Self {
        Self {
            system: Arc::new(system),
        }
    }

    /// The tools this machine can answer, each with its annotations and
    /// output schema. A destructive tool is never among them.
    #[must_use]
    pub fn tools(&self) -> Vec<Tool> {
        mcp_tools::listed(&*self.system)
            .into_iter()
            .map(tool_of)
            .collect()
    }

    /// One call. A tool that fails is an `isError` result (the MCP
    /// convention), never a protocol error; a tool this session does not
    /// list is one too -- calling it would answer a question the listing
    /// said this host cannot.
    #[must_use]
    pub fn call(&self, name: &str) -> CallToolResult {
        let listed = mcp_tools::listed(&*self.system);
        let Some(tool) = listed.into_iter().find(|t| t.name() == name) else {
            return failure(format!("no such tool on this host: {name}"));
        };
        let ran =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| tool.run(&*self.system)));
        match ran {
            Ok(Ok(value)) => success(value),
            Ok(Err(why)) => failure(why),
            Err(_) => failure(format!("{name}: the tool panicked")),
        }
    }

    /// `server/discover`'s `_meta` beyond the server's identity: each
    /// section the Ops view reads that this host does not list, and why.
    #[must_use]
    pub fn absent(&self) -> Value {
        mcp_tools::absent(&*self.system)
    }
}

fn identity() -> Implementation {
    Implementation::new(SERVER_NAME, crate::consts::AGENT_VERSION)
}

fn tool_of(t: mcp_tools::Tool) -> Tool {
    let object = |v: Value| -> JsonObject {
        match v {
            Value::Object(o) => o,
            _ => JsonObject::new(),
        }
    };
    let effect = t.effect();
    let annotations = ToolAnnotations::new()
        .read_only(effect == Effect::Read)
        .destructive(effect == Effect::Destructive)
        .idempotent(effect != Effect::Destructive)
        // launchd and the local Docker daemon: this machine is the world.
        .open_world(false);
    let input =
        serde_json::json!({ "type": "object", "properties": {}, "additionalProperties": false });
    let mut tool = Tool::new(t.name(), t.description(), object(input));
    tool.output_schema = Some(Arc::new(object(t.output_schema())));
    tool.annotations = Some(annotations);
    tool
}

/// A successful reply: the object as text for clients that read text, and
/// the same object as `structuredContent`. The schema promises an object,
/// so anything else is wrapped rather than sent in a shape it forbids.
fn success(value: Value) -> CallToolResult {
    let structured = match value {
        Value::Object(_) => value,
        other => serde_json::json!({ "result": other }),
    };
    let mut result = CallToolResult::success(vec![ContentBlock::text(structured.to_string())]);
    result.structured_content = Some(structured);
    result
}

fn failure(message: String) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(message)])
}

impl<S: System + Send + Sync + 'static> ServerHandler for Server<S> {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(identity())
    }

    // Each answer is ready when asked: `std::future::ready` rather than an
    // `async fn` with nothing to await (clippy `unused_async`).
    fn discover(
        &self,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<DiscoverResult, ErrorData>> + Send + '_ {
        let mut d = DiscoverResult::from_server_info(
            ProtocolVersion::KNOWN_VERSIONS.to_vec(),
            self.get_info(),
        );
        d.ttl_ms = LIST_TTL_MS;
        d.cache_scope = CacheScope::Private;
        d.meta
            .get_or_insert_default()
            .insert(META_ABSENT.to_string(), self.absent());
        std::future::ready(Ok(d))
    }

    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListToolsResult, ErrorData>> + Send + '_ {
        std::future::ready(Ok(ListToolsResult {
            tools: self.tools(),
            ttl_ms: Some(LIST_TTL_MS),
            cache_scope: Some(CacheScope::Private),
            ..ListToolsResult::default()
        }))
    }

    // A call blocks this thread on purpose: one client on a local pipe, and
    // each tool is one `launchctl` or one Docker request.
    fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<CallToolResponse, ErrorData>> + Send + '_ {
        std::future::ready(Ok(self.call(&request.name).into()))
    }
}

/// Serve on stdin/stdout until EOF.
///
/// # Errors
///
/// The runtime could not start, or the session failed.
pub fn run() -> std::io::Result<()> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        let service = Server::new(mcp_tools::Live::new())
            .serve(rmcp::transport::stdio())
            .await
            .map_err(std::io::Error::other)?;
        service.waiting().await.map_err(std::io::Error::other)?;
        Ok(())
    })
}

#[cfg(test)]
#[path = "mcp_tests.rs"]
mod tests;
