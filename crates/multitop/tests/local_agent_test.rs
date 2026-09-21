//! The local-agent path, which does **not** use `ssh` at all.
//!
//! `spawn_local_agent` execs the agent binary directly; that is the whole
//! distinction. These tests were `#[ignore]`d as "requires ssh binary in
//! PATH" -- and `ssh` is present on every machine that has ever run them, so
//! the stated precondition was satisfied while the tests still failed. What
//! they actually need is an agent the test binary can reach, and they now
//! name one: this build's `multitop`, through `MULTITOP_AGENT_EXE` (see
//! `common`). Un-ignored 2026-09-20; while ignored they had rotted past the
//! `Hello` frame the stream now opens with.
//!
//! The "requires ssh" label was the same mistake the seventh review pass fixed
//! in `stream.rs`, where a local panel's missing agent binary was reported as
//! "ssh command not found" -- surviving here, in the metadata of the tests
//! that exercise that very path.

mod common;

use multitop::ssh::{spawn_local_agent, Mode};
use multitop_agent::proto::{decode_packet, Payload};
use multitop_agent::SortBy;
use tokio::io::AsyncReadExt;
use tokio::io::BufReader;

/// One framed packet off the agent's stdout, length-driven like the client.
///
/// A helper is outside clippy's test exemption, so it reports rather than
/// panics; the `#[test]` caller unwraps with the reason.
async fn read_packet<R: tokio::io::AsyncRead + Unpin>(reader: &mut R) -> Result<Payload, String> {
    let mut header = [0u8; 8];
    reader
        .read_exact(&mut header)
        .await
        .map_err(|e| format!("read header: {e}"))?;
    if &header[..4] != multitop_agent::proto::MAGIC {
        return Err(format!("bad magic: {:?}", &header[..4]));
    }

    let payload_len = u16::from_le_bytes([header[6], header[7]]) as usize;
    if payload_len == 0 {
        return Err("empty payload".to_string());
    }

    let mut full = header.to_vec();
    full.resize(8 + payload_len, 0);
    reader
        .read_exact(&mut full[8..])
        .await
        .map_err(|e| format!("read payload: {e}"))?;
    decode_packet(&full).ok_or_else(|| "packet did not decode".to_string())
}

/// The stream opens with `Hello` -- the agent's version, so a mismatch is
/// reported before a snapshot is ever drawn -- and the snapshots follow.
#[tokio::test]
async fn local_agent_streams_binary_packets() {
    common::use_this_builds_agent();
    let mut child = spawn_local_agent(Mode::Monitor, SortBy::Cpu)
        .expect("spawn this build's multitop as its own agent");
    let stdout = child.stdout.take().expect("stdout piped");
    let mut reader = BufReader::new(stdout);

    match read_packet(&mut reader).await.expect("first packet") {
        Payload::Hello(hello) => assert_eq!(
            hello.agent_version,
            multitop_agent::consts::AGENT_VERSION,
            "this build's own binary is the agent, so the versions cannot differ"
        ),
        other => panic!("expected the Hello frame first, got {other:?}"),
    }
    match read_packet(&mut reader).await.expect("second packet") {
        Payload::Monitor(snap) => assert_ne!(snap.host, ""),
        other => panic!("expected a Monitor snapshot after Hello, got {other:?}"),
    }

    let _ = child.kill().await;
}

#[tokio::test]
async fn connect_local_server_succeeds_and_streams_snapshots() {
    use multitop::config::Server;
    use multitop::stream::{connect, next_packet};

    let server = Server {
        host: "localhost".into(),
        port: 0,
        user: String::new(),
        upgrade_cmd: None,
        custom_command: None,
    };

    let mut stream = connect(&server, Mode::Monitor, SortBy::Cpu, |_| {})
        .await
        .expect("connect to the local server -- NotFound here means multitop-agent is not built");

    let mut errbuf = Vec::new();
    let hello = next_packet(&mut stream, &mut errbuf)
        .await
        .expect("read packet")
        .expect("payload present");
    assert!(
        matches!(hello, Payload::Hello(_)),
        "the stream opens with Hello, got {hello:?}"
    );
    let payload = next_packet(&mut stream, &mut errbuf)
        .await
        .expect("read packet")
        .expect("payload present");

    if let Payload::Monitor(snap) = payload {
        assert!(
            !snap.host.is_empty(),
            "local snapshot host should not be empty"
        );
        let rendered = multitop_agent::render::render(
            &snap,
            80,
            24,
            multitop_agent::render::bar_len_for(80),
            &multitop_agent::color::ANSI,
        );
        assert!(
            rendered.iter().any(|l| l.contains("CPU")),
            "frame should contain CPU metric header"
        );
        assert!(
            rendered.iter().any(|l| l.contains("MEM")),
            "frame should contain MEM metric header"
        );
        assert!(
            rendered.iter().any(|l| l.contains("DSK")),
            "frame should contain DSK metric header"
        );
    } else {
        panic!("expected Monitor payload from local server");
    }
}
