//! SSH stream protocol: packet framing, agent bootstrap, and connection.

mod prod;

#[cfg(test)]
#[path = "stream_tests.rs"]
#[allow(clippy::module_inception)]
mod stream_tests;

pub use prod::{
    bootstrap, connect, describe_failure, framing_lost, interpret_packet, next_packet, note,
    read_handshake, spawn_failure, Bootstrap, Handshake, PacketStream, MAX_STDERR_LINES,
};
