//! Why a request did not produce an answer.
//!
//! The client's own vocabulary for a failure, and the ONE place that
//! decides what a failure MEANS: a deadline missed and a pipe that closed
//! are a GAP in the reading (the estate said nothing, so this client
//! knows nothing about the thing it asked), and the estate's own report
//! of a failure is a FINDING (servers ROADMAP E74).
//!
//! It is its own module because it is the seam every other part of the
//! client reads: the request machinery raises it, the gather sorts it into
//! "the answer did not arrive" and "the estate reported a failure", and
//! the frame draws the two differently.

use std::fmt;
use std::time::Duration;

use rmcp::service::ClientInitializeError;
use rmcp::ServiceError;

use super::client::PROTOCOL_VERSION;

/// Why a request did not produce an answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The pipe closed or broke: the server exited, or ssh did.
    Closed(String),
    /// No reply within the deadline: WHAT was asked, and how long it waited.
    ///
    /// The subject is the tool (or the resource), not the JSON-RPC method:
    /// `tools/call` names a method and no tool at all, so an operator
    /// reading "tools/call: no answer in 20s" could not tell which call
    /// had missed the deadline (servers ROADMAP E74).
    Timeout { what: String, after: Duration },
    /// A JSON-RPC error reply.
    Rpc { code: i64, message: String },
    /// The tool ran and reported failure (`isError`), with its text.
    Tool(String),
    /// The server does not offer 2026-07-28; these are what it offers.
    Unsupported(Vec<String>),
    /// A reply this client cannot read.
    Protocol(String),
}

impl Error {
    /// What rmcp's failure of one request MEANS here; `what` is the call.
    pub(super) fn of_service(e: ServiceError, what: &str) -> Self {
        match e {
            ServiceError::McpError(e) => Self::Rpc {
                code: i64::from(e.code.0),
                message: e.message.into_owned(),
            },
            ServiceError::TransportClosed | ServiceError::TransportSend(_) => {
                Self::Closed("end of output".to_string())
            }
            ServiceError::Timeout { timeout } => Self::Timeout {
                what: what.to_string(),
                after: timeout,
            },
            other => Self::Protocol(format!("{what}: {other}")),
        }
    }

    /// What a failure to open the session MEANS here.
    pub(super) fn of_opening(e: ClientInitializeError) -> Self {
        match e {
            ClientInitializeError::NoCompatibleProtocolVersion {
                server_supported, ..
            } => Self::Unsupported(server_supported.iter().map(ToString::to_string).collect()),
            ClientInitializeError::JsonRpcError(e) => Self::Rpc {
                code: i64::from(e.code.0),
                message: e.message.into_owned(),
            },
            ClientInitializeError::ConnectionClosed(why) => Self::Closed(why),
            ClientInitializeError::TransportError { error, .. } => Self::Closed(error.to_string()),
            other => Self::Protocol(format!("server/discover: {other}")),
        }
    }
}

impl Error {
    /// Whether this failure means the ANSWER WAS NOT READ.
    ///
    /// A deadline missed and a pipe that closed are both a GAP in the
    /// reading: the estate said nothing about the thing that was asked,
    /// so this client knows nothing about it. The estate's OWN report of
    /// a failure is not a gap - that is a finding, and it is the finding
    /// (servers ROADMAP E74).
    #[must_use]
    pub const fn is_unread(&self) -> bool {
        matches!(self, Self::Closed(_) | Self::Timeout { .. })
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Closed(why) => write!(f, "mcp_host closed the session: {why}"),
            Self::Timeout { what, after } if after.as_secs() == 0 => {
                write!(f, "{what}: no answer in {}ms", after.as_millis())
            }
            Self::Timeout { what, after } => {
                write!(f, "{what}: no answer in {}s", after.as_secs())
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
