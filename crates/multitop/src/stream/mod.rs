//! SSH stream protocol: packet framing, agent bootstrap, and connection.

mod prod;

#[cfg(test)]
#[path = "stream_tests.rs"]
#[allow(clippy::module_inception)]
mod stream_tests;

pub use prod::{connect, next_packet, read_handshake, Handshake, PacketStream};
