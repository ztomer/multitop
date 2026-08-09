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

#[tokio::test]
async fn a_framed_packet_is_read_and_decoded() {
    let mut stream = stream_from_bytes(&packet("web-01"), b"");
    let mut errbuf = Vec::new();

    let payload = next_packet(&mut stream, &mut errbuf)
        .await
        .expect("read must succeed")
        .expect("a payload must be there");
    let Payload::Monitor(snap) = payload else {
        panic!("wrong payload kind");
    };
    assert_eq!(snap.host, "web-01");
    assert!(
        errbuf.is_empty(),
        "a clean read must report nothing: {errbuf:?}"
    );
}

#[tokio::test]
async fn packets_are_read_back_to_back_until_the_stream_ends() {
    let mut bytes = packet("a");
    bytes.extend_from_slice(&packet("b"));
    let mut stream = stream_from_bytes(&bytes, b"");
    let mut errbuf = Vec::new();

    for expected in ["a", "b"] {
        let Some(Payload::Monitor(snap)) = next_packet(&mut stream, &mut errbuf).await.unwrap()
        else {
            panic!("expected a monitor packet");
        };
        assert_eq!(snap.host, expected);
    }
    // End of stream is `None`, not an error — the caller turns that into a
    // closed connection.
    assert!(next_packet(&mut stream, &mut errbuf)
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn a_header_the_handshake_already_consumed_is_not_read_twice() {
    // `connect` reads four bytes to recognise the framing and hands them back
    // through `pending_header`; reading eight more here would take the first
    // half of the payload as the rest of the header.
    let mut stream = stream_from_bytes(&packet("web-01")[4..], b"");
    stream.pending_header = Some(*multitop_agent::proto::MAGIC);
    let mut errbuf = Vec::new();

    let Some(Payload::Monitor(snap)) = next_packet(&mut stream, &mut errbuf).await.unwrap() else {
        panic!("expected a monitor packet");
    };
    assert_eq!(snap.host, "web-01");
}

#[tokio::test]
async fn a_stream_that_ends_inside_a_pending_header_is_a_close_not_an_error() {
    let mut stream = stream_from_bytes(&[0x00, 0x01], b"");
    stream.pending_header = Some(*multitop_agent::proto::MAGIC);
    let mut errbuf = Vec::new();
    assert!(next_packet(&mut stream, &mut errbuf)
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn a_stream_that_produces_nothing_at_all_is_a_close() {
    let mut stream = stream_from("", "");
    let mut errbuf = Vec::new();
    assert!(next_packet(&mut stream, &mut errbuf)
        .await
        .unwrap()
        .is_none());
    assert_eq!(errbuf, [] as [std::string::String; 0]);
}

#[tokio::test]
async fn text_where_the_framing_should_be_is_reported_as_that() {
    // The message has to say the agent never started, not that the connection
    // closed: the host is up and talking.
    let mut stream = stream_from("Welcome to Ubuntu 22.04\n", "");
    stream.preamble = Some("Welcome to Ubuntu 22.04".to_string());
    let mut errbuf = Vec::new();

    let err = next_packet(&mut stream, &mut errbuf).await.unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    assert!(
        errbuf.iter().any(|l| l.contains("Welcome to Ubuntu")),
        "{errbuf:?}"
    );
    assert!(
        errbuf.iter().any(|l| l.contains("login banner")),
        "{errbuf:?}"
    );
}

#[tokio::test]
async fn framing_lost_mid_stream_is_reported_as_that() {
    // A good packet, then eight bytes that are not a header. No preamble, so
    // this is the "was working, then desynchronised" wording.
    let mut bytes = packet("web-01");
    bytes.extend_from_slice(b"NOTMAGIC");
    let mut stream = stream_from_bytes(&bytes, b"");
    let mut errbuf = Vec::new();

    assert!(next_packet(&mut stream, &mut errbuf)
        .await
        .unwrap()
        .is_some());
    let err = next_packet(&mut stream, &mut errbuf).await.unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    assert!(
        errbuf.iter().any(|l| l.contains("framing was lost")),
        "{errbuf:?}"
    );
    assert!(
        errbuf.iter().any(|l| l.contains("host is reachable")),
        "the operator must not be sent to look at the network: {errbuf:?}"
    );
}

#[tokio::test]
async fn a_payload_cut_short_is_an_error_the_panel_can_report() {
    // Header promises a payload; the stream stops halfway through it.
    let full = packet("web-01");
    let mut stream = stream_from_bytes(&full[..full.len() - 4], b"");
    let mut errbuf = Vec::new();

    let err = next_packet(&mut stream, &mut errbuf).await.unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::UnexpectedEof);
    assert!(
        errbuf
            .iter()
            .any(|l| l.contains("reading from the agent failed")),
        "a truncated payload must leave a reason behind: {errbuf:?}"
    );
}

#[tokio::test]
async fn stderr_that_arrives_while_waiting_for_a_header_is_kept() {
    // The remote writes a warning and then nothing else. The reader must
    // collect the warning rather than blocking on stdout with it unread.
    let mut stream = stream_from("", "Warning: Permanently added a host\n");
    let mut errbuf = Vec::new();

    assert!(next_packet(&mut stream, &mut errbuf)
        .await
        .unwrap()
        .is_none());
    assert!(
        errbuf.iter().any(|l| l.contains("Permanently added")),
        "stderr arriving before EOF must be kept: {errbuf:?}"
    );
}

// ------------------------------------------------------------ the error buffer

#[test]
fn the_reason_buffer_keeps_the_most_recent_lines_and_no_more() {
    let mut errbuf = Vec::new();
    for i in 0..MAX_STDERR_LINES * 2 {
        note(&mut errbuf, format!("line {i}"));
    }
    assert_eq!(errbuf.len(), MAX_STDERR_LINES, "the bound was not applied");
    // The oldest are dropped, so the last thing that happened survives.
    assert_eq!(
        errbuf.last().unwrap(),
        &format!("line {}", MAX_STDERR_LINES * 2 - 1)
    );
    assert!(!errbuf.iter().any(|l| l == "line 0"));
}

#[test]
fn a_framing_failure_is_described_without_being_prefixed() {
    // `framing_lost` already says what happened and what to do; wrapping it
    // would bury that behind a second sentence.
    let e = framing_lost(b"NOTMAGIC", None);
    assert_eq!(describe_failure(&e), e.to_string());

    let with_text = framing_lost(b"Welcome!", Some("Welcome to Ubuntu".into()));
    assert!(with_text.to_string().contains("Welcome to Ubuntu"));
    assert!(with_text.to_string().contains("never started"));
}

#[test]
fn any_other_read_failure_says_the_host_may_still_be_up() {
    let e = std::io::Error::new(std::io::ErrorKind::BrokenPipe, "pipe went away");
    let described = describe_failure(&e);
    assert!(described.contains("reading from the agent failed"));
    assert!(described.contains("pipe went away"));
    assert!(described.contains("may still be reachable"));
}

#[test]
fn a_packet_this_build_cannot_read_is_not_reported_as_a_closed_connection() {
    let mut errbuf = Vec::new();
    // Intact framing, unknown mode byte: the stream is fine, the dialect is not.
    let mut pkt = packet("web-01");
    pkt[5] = 99;
    assert!(interpret_packet(&pkt, 99, pkt.len() - 8, &mut errbuf).is_none());
    assert!(
        errbuf.iter().any(|l| l.contains("different version")),
        "{errbuf:?}"
    );
    assert!(errbuf.iter().any(|l| l.contains("mode 99")), "{errbuf:?}");

    // A packet that decodes leaves no complaint behind.
    let mut clean = Vec::new();
    let good = packet("web-01");
    assert!(interpret_packet(&good, good[5], good.len() - 8, &mut clean).is_some());
    assert_eq!(clean, [] as [std::string::String; 0]);
}

// -------------------------------------------------------------- the handshake

#[tokio::test]
async fn the_handshake_recognises_framing_split_across_reads() {
    // A pipe may hand back fewer bytes than asked for. Comparing what arrived
    // instead of the whole header made the agent's own framing look like text.
    for chunk in [1usize, 2, 3, 4] {
        let magic = multitop_agent::proto::MAGIC;
        let mut reader = ChunkedReader::new(magic.to_vec(), chunk);
        assert_eq!(
            read_handshake(&mut reader).await,
            Handshake::Framed,
            "chunk={chunk}"
        );
    }
}

#[tokio::test]
async fn the_handshake_reads_a_banner_as_text() {
    let mut reader = ChunkedReader::new(b"Welcome to Ubuntu 22.04 LTS\n".to_vec(), 3);
    assert_eq!(
        read_handshake(&mut reader).await,
        Handshake::Text("Welcome to Ubuntu 22.04 LTS".to_string())
    );
}

#[tokio::test]
async fn a_session_that_says_nothing_is_a_close() {
    let mut reader = ChunkedReader::new(Vec::new(), 4);
    assert_eq!(read_handshake(&mut reader).await, Handshake::Closed);
    // Fewer than four bytes is the same: there is no header there.
    let mut short = ChunkedReader::new(b"hi".to_vec(), 1);
    assert_eq!(read_handshake(&mut short).await, Handshake::Closed);
}

/// A reader that hands back at most `chunk` bytes per call, like a pipe.
struct ChunkedReader {
    data: Vec<u8>,
    pos: usize,
    chunk: usize,
}

impl ChunkedReader {
    const fn new(data: Vec<u8>, chunk: usize) -> Self {
        Self {
            data,
            pos: 0,
            chunk,
        }
    }
}

impl tokio::io::AsyncRead for ChunkedReader {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let take = self
            .chunk
            .min(buf.remaining())
            .min(self.data.len() - self.pos);
        let (pos, chunk) = (self.pos, take);
        buf.put_slice(&self.data[pos..pos + chunk]);
        self.pos += take;
        std::task::Poll::Ready(Ok(()))
    }
}

impl tokio::io::AsyncBufRead for ChunkedReader {
    fn poll_fill_buf(
        self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<&[u8]>> {
        let this = self.get_mut();
        let end = (this.pos + this.chunk).min(this.data.len());
        std::task::Poll::Ready(Ok(&this.data[this.pos..end]))
    }
    fn consume(mut self: std::pin::Pin<&mut Self>, amt: usize) {
        self.pos += amt;
    }
}

// --------------------------------------------------------------- the decision

#[test]
fn framing_at_the_handshake_starts_the_reader_with_the_header_it_already_read() {
    assert_eq!(
        bootstrap(Handshake::Framed, "web-01", 0, None),
        Bootstrap::Ready {
            pending_header: Some(*multitop_agent::proto::MAGIC),
            preamble: None,
        }
    );
}

#[test]
fn text_at_the_handshake_is_carried_to_whoever_reports_the_failure() {
    assert_eq!(
        bootstrap(Handshake::Text("banner".into()), "web-01", 0, None),
        Bootstrap::Ready {
            pending_header: None,
            preamble: Some("banner".into()),
        }
    );
}

#[test]
fn a_closed_session_reports_what_stderr_said_when_it_said_anything() {
    assert_eq!(
        bootstrap(
            Handshake::Closed,
            "web-01",
            0,
            Some("Permission denied".into())
        ),
        Bootstrap::Failed("Permission denied".into())
    );
    // With nothing on stderr, all that can honestly be said is that it closed.
    assert_eq!(
        bootstrap(Handshake::Closed, "web-01", 0, None),
        Bootstrap::Failed("Connection to web-01 closed".into())
    );
}

#[test]
fn a_remote_with_no_agent_asks_for_the_build_that_matches_it() {
    assert_eq!(
        bootstrap(Handshake::NeedAgent("x86_64".into()), "h", 0, None),
        Bootstrap::Install(Arch::X86_64)
    );
    assert_eq!(
        bootstrap(Handshake::NeedAgent("aarch64".into()), "h", 0, None),
        Bootstrap::Install(Arch::Aarch64)
    );
}

#[test]
fn an_architecture_multitop_does_not_ship_is_named_in_the_failure() {
    let Bootstrap::Failed(msg) =
        bootstrap(Handshake::NeedAgent("mips64".into()), "web-01", 0, None)
    else {
        panic!("an unshippable architecture must fail");
    };
    assert!(msg.contains("mips64"), "{msg}");
    assert!(msg.contains("web-01"), "{msg}");
    assert!(msg.contains("x86_64 and aarch64"), "{msg}");
}

#[test]
fn an_agent_that_is_still_missing_after_an_install_is_not_installed_again() {
    // Second pass: asking for the agent again means the install did not take,
    // and retrying forever would peg the host.
    assert_eq!(
        bootstrap(Handshake::NeedAgent("x86_64".into()), "web-01", 1, None),
        Bootstrap::Failed("Agent did not start on web-01 after install".into())
    );
}

// ---------------------------------------------------------------- spawn errors

#[test]
fn a_missing_binary_names_the_program_that_is_actually_missing() {
    let local = Server {
        host: "localhost".into(),
        port: 0,
        user: String::new(),
        upgrade_cmd: None,
    };
    let remote = Server {
        host: "web-01".into(),
        port: 22,
        user: "root".into(),
        upgrade_cmd: None,
    };
    let not_found = std::io::Error::new(std::io::ErrorKind::NotFound, "no such file");

    // A local panel never runs `ssh`, so telling the operator to go find one
    // sends them after a program that was working the whole time.
    assert!(spawn_failure(&local, &not_found).contains("multitop-agent binary was not found"));
    assert_eq!(spawn_failure(&remote, &not_found), "ssh command not found");
}

#[test]
fn any_other_spawn_failure_still_names_the_program() {
    let local = Server {
        host: "localhost".into(),
        port: 0,
        user: String::new(),
        upgrade_cmd: None,
    };
    let remote = Server {
        host: "web-01".into(),
        port: 22,
        user: "root".into(),
        upgrade_cmd: None,
    };
    let denied = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied");

    assert!(spawn_failure(&local, &denied).starts_with("could not start multitop-agent"));
    assert!(spawn_failure(&remote, &denied).starts_with("ssh: "));
}

#[tokio::test]
async fn the_reason_on_stderr_survives_stdout_closing_first() {
    // The regression this exists for. `select!` raced the EOF against the
    // stderr line and returned on whichever landed first, so the diagnosis was
    // kept or thrown away depending on how loaded the machine was -- it passed
    // under `cargo test` and failed under the slower instrumented run.
    //
    // Written so stdout closes *before* stderr is written, which is the losing
    // order, and repeated so a single lucky scheduling cannot pass it.
    // `exec 1>&-` closes stdout before anything is written to stderr, so the
    // reader sees EOF first every time. That is the losing order, made certain
    // rather than waited for: as a race it passed under `cargo test` and failed
    // under the slower instrumented run.
    let mut stream =
        stream_from_script("exec 1>&-; sleep 0.05; printf 'Permission denied (publickey)\\n' >&2");
    let mut errbuf = Vec::new();

    assert!(next_packet(&mut stream, &mut errbuf)
        .await
        .unwrap()
        .is_none());
    assert!(
        errbuf.iter().any(|l| l.contains("Permission denied")),
        "the close was reported with no reason: {errbuf:?}"
    );
}

#[tokio::test]
async fn a_close_with_a_silent_stderr_is_still_a_close() {
    // Draining must not turn "nothing to say" into a wait.
    let mut stream = stream_from("", "");
    let mut errbuf = Vec::new();
    assert!(next_packet(&mut stream, &mut errbuf)
        .await
        .unwrap()
        .is_none());
    assert!(
        errbuf.is_empty(),
        "a silent close invented a reason: {errbuf:?}"
    );
}
