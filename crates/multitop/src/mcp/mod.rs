//! A host's `mcp_host`, spoken to over the host's ssh session (servers
//! ROADMAP 12.17b): the protocol in `client`, the process in `spawn`.

pub mod catalogue;
pub mod client;
pub mod error;
pub mod spawn;

#[cfg(test)]
#[path = "fake_tests.rs"]
pub mod fake;

#[cfg(test)]
#[path = "catalogue_tests.rs"]
mod catalogue_tests;

#[cfg(test)]
#[path = "catalogue_client_tests.rs"]
mod catalogue_client_tests;

#[cfg(test)]
#[path = "catalogue_digest_tests.rs"]
mod catalogue_digest_tests;

#[cfg(test)]
#[path = "client_tests.rs"]
mod tests;
