//! A host's `mcp_host`, spoken to over the host's ssh session (servers
//! ROADMAP 12.17b): the protocol in `client`, the process in `spawn`.

pub mod client;
pub mod spawn;

#[cfg(test)]
#[path = "fake_tests.rs"]
pub mod fake;

#[cfg(test)]
#[path = "client_tests.rs"]
mod tests;
