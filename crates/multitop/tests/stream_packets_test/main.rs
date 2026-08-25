//! The packet reader, driven against a real child process.
//!
//! `PacketStream` owns a live child, which is why this path went untested: the
//! agent has to be built and reachable before `connect` will produce one. A
//! shell that writes canned bytes is a child just the same, and it can be told
//! to split a header, interleave stderr, or stop mid-payload on cue.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::process::Stdio;

use multitop::config::Server;
use multitop::ssh::Arch;
use multitop::stream::{
    bootstrap, describe_failure, framing_lost, interpret_packet, next_packet, note, read_handshake,
    spawn_failure, Bootstrap, Handshake, PacketStream, MAX_STDERR_LINES,
};
use multitop_agent::proc::Usage;
use multitop_agent::proto::{encode_packet, Payload};
use multitop_agent::render::Snapshot;
use tokio::io::{AsyncBufReadExt as _, BufReader};
use tokio::process::Command;

fn snapshot(host: &str) -> Snapshot {
    Snapshot {
        host: host.into(),
        agent_version: "9.9.9".into(),
        cpu_pct: 25.0,
        mem: Usage::new(8 << 30, 2 << 30),
        disk: Usage::new(256 << 30, 64 << 30),
        ..Default::default()
    }
}

fn packet(host: &str) -> Vec<u8> {
    encode_packet(&Payload::Monitor(snapshot(host)))
}

/// A `PacketStream` fed by a shell that writes `stdout_script` to stdout and
/// `stderr_script` to stderr. Both are `printf` format strings, so `\\xNN`
/// escapes put arbitrary bytes on the wire.
/// A stream over exact bytes on stdout and stderr.
///
/// The bytes go through files and `cat`, with no shell escaping anywhere,
/// because there is no portable way to write a byte in a `printf` format
/// string. These used to be `printf '\x4d\x54...'` -- a hex escape bash and
/// macOS `printf` accept and dash, which is `/bin/sh` on most Linux, does not.
/// On the runner the reader was handed the literal text `\x4d\x54` and
/// reported the agent's framing as lost, which is exactly what it should say
/// about a stream carrying that. Six tests, only ever red on Linux, and nothing
/// local could see it.
fn stream_from_bytes(stdout: &[u8], stderr: &[u8]) -> PacketStream {
    // Leaked on purpose: the child reads them after this returns, and the
    // handful of small files a test run makes are cleaned up by the OS. A
    // `TempDir` would have to outlive the `PacketStream`, which the callers
    // have no way to hold.
    let dir = Box::leak(Box::new(tempfile::tempdir().expect("tempdir")));
    let out = dir.path().join("out");
    let err = dir.path().join("err");
    std::fs::write(&out, stdout).expect("write stdout");
    std::fs::write(&err, stderr).expect("write stderr");
    stream_from_script(&format!(
        "cat {o}; cat {e} >&2",
        o = out.display(),
        e = err.display()
    ))
}

/// The same, for the cases whose payload is plain text.
fn stream_from(stdout: &str, stderr: &str) -> PacketStream {
    stream_from_bytes(stdout.as_bytes(), stderr.as_bytes())
}

/// A stream over a shell script written out in full, for the cases that need to
/// control the *order* stdout and stderr close in.
fn stream_from_script(script: &str) -> PacketStream {
    let script = script.to_string();
    let mut child = Command::new("sh")
        .arg("-c")
        .arg(script)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("spawn sh");

    let stdout = BufReader::new(child.stdout.take().unwrap());
    let stderr = BufReader::new(child.stderr.take().unwrap());
    PacketStream {
        child,
        stdout,
        stderr: stderr.lines(),
        pending_header: None,
        preamble: None,
    }
}

// ------------------------------------------------------------ packet reading
mod handshake_and_install;
mod packet_reading;
