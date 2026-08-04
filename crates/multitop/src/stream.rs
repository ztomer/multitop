//! SSH stream protocol: packet framing, agent bootstrap, and connection.

use tokio::io::{BufReader, Lines};
use tokio::process::{Child, ChildStderr, ChildStdout};

use crate::config::Server;
use crate::fmt::status_line;
use crate::ssh::{self, Arch, Mode};

use multitop_agent::SortBy;

/// Max stderr lines retained for the failure message when a connection dies.
const MAX_STDERR_LINES: usize = 8;

pub struct PacketStream {
    pub child: Child,
    pub stdout: BufReader<ChildStdout>,
    pub stderr: Lines<BufReader<ChildStderr>>,
    pub pending_header: Option<[u8; 4]>,
    /// What came out of stdout before the framing did, when it was not framing.
    ///
    /// A login banner, a shell profile that prints on a non-interactive
    /// session, an error from the remote's rc files. [`read_handshake`] reads
    /// that line to find out what it is; kept here, it is available to say what
    /// went wrong when the packet reader then fails on it. Dropped, the panel
    /// could only report a connection that was never closed.
    pub preamble: Option<String>,
}

/// What the first bytes of a session turned out to be.
#[derive(Debug, PartialEq, Eq)]
pub enum Handshake {
    /// The agent is running and the stream is already framed.
    Framed,
    /// The remote has no usable agent for the architecture it named.
    NeedAgent(String),
    /// Something else came out first -- a banner, an error. Left to the packet
    /// reader to fail on, and it carries the text so that failure can name it.
    ///
    /// The text used to be dropped here, which made the promise above false:
    /// the reader failed on the *next* eight bytes with `invalid magic header`,
    /// every caller turned that into `Connection to <host> closed`, and the one
    /// line that said what was actually wrong had already been thrown away.
    Text(String),
    /// Nothing came out at all.
    Closed,
}

/// Read the first line of a session and decide what it was.
///
/// `read_exact`, not `read`. A pipe is free to hand back fewer bytes than were
/// asked for, and a magic header split across two reads was compared four bytes
/// at a time against a buffer holding one or two: the agent's own framing was
/// then mistaken for text, the rest of the line consumed looking for a
/// newline, and every packet after it read from the wrong offset. The panel
/// showed `invalid magic header` and reconnected -- against a host that was
/// working perfectly.
///
/// Separated from [`connect`] so it can be given a reader that splits where a
/// real pipe might, which is the only way to see that defect on purpose.
pub async fn read_handshake<R>(stdout: &mut R) -> Handshake
where
    R: tokio::io::AsyncBufRead + Unpin,
{
    use tokio::io::{AsyncBufReadExt, AsyncReadExt};

    let mut first4 = [0u8; 4];
    if stdout.read_exact(&mut first4).await.is_err() {
        return Handshake::Closed;
    }
    if &first4 == multitop_agent::proto::MAGIC {
        return Handshake::Framed;
    }
    let mut line = String::from_utf8_lossy(&first4).to_string();
    let mut rest = String::new();
    let _ = stdout.read_line(&mut rest).await;
    line.push_str(&rest);
    ssh::parse_need_agent(&line).map_or_else(
        || Handshake::Text(line.trim_end().to_string()),
        |arch| Handshake::NeedAgent(arch.to_string()),
    )
}

/// Say which program could not be started.
///
/// A local panel does not run `ssh` at all -- it spawns the agent binary
/// directly -- so mapping every `NotFound` to "ssh command not found" told the
/// user to go looking for an `ssh` that was installed and working the whole
/// time. The two spawn paths fail for different reasons and have different
/// fixes; the message has to name the one that happened.
fn spawn_failure(server: &Server, e: &std::io::Error) -> String {
    if e.kind() == std::io::ErrorKind::NotFound {
        return if ssh::is_local(server) {
            "the multitop-agent binary was not found next to multitop or on PATH".to_string()
        } else {
            "ssh command not found".to_string()
        };
    }
    if ssh::is_local(server) {
        format!("could not start multitop-agent: {e}")
    } else {
        format!("ssh: {e}")
    }
}

/// Connect to a remote server over SSH and bootstrap the agent if needed.
///
/// # Errors
///
/// Returns an error string if SSH execution fails or the agent cannot be started.
pub async fn connect(
    server: &Server,
    mode: Mode,
    sort: SortBy,
    on_status: impl Fn(String),
) -> Result<PacketStream, String> {
    use tokio::io::AsyncBufReadExt;

    for attempt in 0..2 {
        let mut child = ssh::spawn_agent(server, mode, sort)
            .await
            .map_err(|e| spawn_failure(server, &e))?;

        let Some(stdout) = child.stdout.take() else {
            return Err("failed to capture stdout".to_string());
        };
        let Some(stderr) = child.stderr.take() else {
            return Err("failed to capture stderr".to_string());
        };
        let mut stdout = BufReader::new(stdout);
        let mut stderr = BufReader::new(stderr).lines();

        let arch_str = match read_handshake(&mut stdout).await {
            Handshake::Closed => {
                let mut detail = String::new();
                while let Ok(Some(l)) = stderr.next_line().await {
                    if !l.trim().is_empty() {
                        detail = l;
                    }
                }
                return Err(if detail.is_empty() {
                    format!("Connection to {} closed", server.host)
                } else {
                    detail
                });
            }
            Handshake::Framed => {
                return Ok(PacketStream {
                    child,
                    stdout,
                    stderr,
                    pending_header: Some(*multitop_agent::proto::MAGIC),
                    preamble: None,
                })
            }
            Handshake::Text(line) => {
                return Ok(PacketStream {
                    child,
                    stdout,
                    stderr,
                    pending_header: None,
                    preamble: Some(line),
                })
            }
            Handshake::NeedAgent(arch) => arch,
        };

        if attempt > 0 {
            return Err(format!(
                "Agent did not start on {} after install",
                server.host
            ));
        }
        let Some(arch) = Arch::from_uname(&arch_str) else {
            return Err(format!(
                "Unsupported architecture '{arch_str}' on {} - multitop ships x86_64 and aarch64",
                server.host
            ));
        };
        on_status(status_line(format!(
            "\u{2192} installing agent ({})...",
            arch.label()
        )));
        let token = format!("{}", std::process::id());
        ssh::upload_agent(server, arch, &token).await?;
    }
    unreachable!("loop returns on both attempts")
}

/// Read next packet from stream, and leave any failure where the panel will
/// find it.
///
/// Every `Err` below reaches the panel by one path, because none of them reach
/// it by any other. All three readers of this stream pattern-match
/// `while let Ok(Some(payload))`, which ends their loop and drops the error;
/// what they report afterwards is built from `errbuf`. The framing failure was
/// given that treatment on its own first, which left its three siblings -- the
/// two short header reads and the payload read -- still silent: an I/O error
/// mid-stream stopped the monitor with `Connection to <host> closed` and
/// stopped fetch and docker with nothing at all. Noting the error here rather
/// than at each `return Err` is what stops the next one being added silently.
///
/// # Errors
///
/// Returns an error if reading from stdout/stderr fails or process exits.
pub async fn next_packet(
    stream: &mut PacketStream,
    errbuf: &mut Vec<String>,
) -> std::io::Result<Option<multitop_agent::proto::Payload>> {
    let outcome = read_packet(stream, errbuf).await;
    if let Err(e) = &outcome {
        note(errbuf, describe_failure(e));
    }
    outcome
}

/// Why the stream stopped, in a line an operator can act on.
fn describe_failure(e: &std::io::Error) -> String {
    if e.kind() == std::io::ErrorKind::InvalidData {
        // Composed by `framing_lost`, which already names the cause and what to
        // do about it. Prefixing it would bury that.
        e.to_string()
    } else {
        format!(
            "reading from the agent failed: {e} -- the host may still be reachable; \
             the session is being restarted"
        )
    }
}

async fn read_packet(
    stream: &mut PacketStream,
    errbuf: &mut Vec<String>,
) -> std::io::Result<Option<multitop_agent::proto::Payload>> {
    use multitop_agent::proto;
    use tokio::io::AsyncReadExt;

    let mut header = [0u8; 8];
    if let Some(pending4) = stream.pending_header.take() {
        header[..4].copy_from_slice(&pending4);
        match stream.stdout.read_exact(&mut header[4..8]).await {
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(e) => return Err(e),
        }
    } else {
        loop {
            tokio::select! {
                res = stream.stdout.read_exact(&mut header) => {
                    match res {
                        Ok(_) => break,
                        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
                        Err(e) => return Err(e),
                    }
                }
                Ok(Some(line)) = stream.stderr.next_line() => {
                    if !line.trim().is_empty() {
                        note(errbuf, line);
                    }
                }
            }
        }
    }

    if &header[..4] != proto::MAGIC {
        return Err(framing_lost(&header, stream.preamble.take()));
    }
    let len = u16::from_le_bytes([header[6], header[7]]) as usize;
    let mut payload_bytes = vec![0u8; len];
    stream.stdout.read_exact(&mut payload_bytes).await?;

    let mut full_packet = Vec::with_capacity(8 + len);
    full_packet.extend_from_slice(&header);
    full_packet.extend_from_slice(&payload_bytes);

    Ok(interpret_packet(&full_packet, header[5], len, errbuf))
}

/// Add a reason to the bounded buffer the panel reports from.
///
/// One bound, in one place. It was written twice with two different rules --
/// `> MAX_STDERR_LINES` on the stderr path and `>= MAX_STDERR_LINES` on the
/// reason path -- so the buffer held nine lines or eight depending on which
/// kind of line arrived last. The same shape as this round's stale panel count,
/// its hit test and its scroll clamp: one quantity, two places, two rules.
fn note(errbuf: &mut Vec<String>, reason: String) {
    if errbuf.len() >= MAX_STDERR_LINES {
        errbuf.remove(0);
    }
    errbuf.push(reason);
}

/// Say why a packet header was not the agent's framing.
///
/// The message is the whole point: it is what [`next_packet`] puts in `errbuf`,
/// which is the only path by which any failure here reaches the panel. Before
/// it existed the monitor reported `Connection to <host> closed` about a host
/// that was up and talking, and the fetch and docker panels reported nothing
/// whatsoever.
///
/// Class H, and the fifth sibling of it this round: a failure reported as
/// something else.
fn framing_lost(header: &[u8], preamble: Option<String>) -> std::io::Error {
    let reason = preamble.map_or_else(
        || {
            format!(
                "the agent's framing was lost mid-stream (expected a packet header, got {:?}) \
                 -- the host is reachable; the session is being restarted",
                String::from_utf8_lossy(header)
            )
        },
        |text| {
            format!(
                "the remote sent text where the agent's framing should be, so the agent never \
                 started -- a login banner or a shell profile that prints on a non-interactive \
                 session will do this. It said: {text}"
            )
        },
    );
    std::io::Error::new(std::io::ErrorKind::InvalidData, reason)
}

/// Turn a framed packet into a payload, or record why it could not be read.
///
/// Split out so the undecodable case can be reached from a test: `next_packet`
/// needs a `PacketStream`, and a `PacketStream` owns a live child process.
fn interpret_packet(
    full_packet: &[u8],
    mode: u8,
    len: usize,
    errbuf: &mut Vec<String>,
) -> Option<multitop_agent::proto::Payload> {
    use multitop_agent::proto;

    let decoded = proto::decode_packet(full_packet);
    if decoded.is_none() {
        // `None` is the caller's word for "the stream ended", and every caller
        // turns it into `Connection to <host> closed`. A packet that arrived
        // intact and could not be read is not a closed connection: it is an
        // agent speaking a dialect this build does not know, and saying
        // "closed" about a host that is up and talking sends the operator to
        // look at the network. The framing stayed aligned -- `len` bytes were
        // read either way -- so the session still ends here on purpose, to make
        // the reconnect re-run the version check that should have caught it.
        note(
            errbuf,
            format!(
                "agent sent a packet this build cannot read (mode {mode}, {len} bytes) \
                 -- the host is reachable; the agent is a different version"
            ),
        );
    }
    decoded
}

#[cfg(test)]
mod spawn_failure_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::spawn_failure;
    use crate::config::Server;

    fn server(host: &str, port: u16) -> Server {
        Server {
            host: host.to_string(),
            port,
            user: "admin".to_string(),
            upgrade_cmd: None,
        }
    }

    /// A local panel never runs `ssh`. Blaming `ssh` for a missing agent binary
    /// sends the user to check an `ssh` that is installed and working.
    #[test]
    fn a_local_panel_blames_the_agent_binary_not_ssh() {
        let missing = std::io::Error::from(std::io::ErrorKind::NotFound);
        let msg = spawn_failure(&server("localhost", 0), &missing);
        assert!(
            msg.contains("multitop-agent"),
            "the message must name what was actually missing: {msg}"
        );
        assert!(
            !msg.contains("ssh"),
            "and must not send the user after ssh: {msg}"
        );
    }

    /// A remote panel does run `ssh`, so that one keeps its message.
    #[test]
    fn a_remote_panel_still_blames_ssh() {
        let missing = std::io::Error::from(std::io::ErrorKind::NotFound);
        let msg = spawn_failure(&server("db-02", 22), &missing);
        assert!(msg.contains("ssh"), "{msg}");
    }

    /// Failures that are not "missing program" keep their detail, and still
    /// name the right program.
    #[test]
    fn other_failures_keep_their_detail() {
        let denied = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        let local = spawn_failure(&server("localhost", 0), &denied);
        assert!(local.contains("multitop-agent"), "{local}");
        let remote = spawn_failure(&server("db-02", 22), &denied);
        assert!(remote.starts_with("ssh:"), "{remote}");
    }
}

#[cfg(test)]
mod packet_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::interpret_packet;
    use multitop_agent::proto;

    /// A packet that arrived intact and could not be read is not a closed
    /// connection.
    ///
    /// It used to become one: `decode_packet` returns `None` for a mode byte
    /// this build does not know, `next_packet` handed that straight back as
    /// `Ok(None)`, and every caller turns `Ok(None)` into
    /// `Connection to <host> closed`. The operator was sent to look at the
    /// network for a host that was up and talking.
    #[test]
    fn an_unreadable_packet_says_so_rather_than_claiming_the_host_went_away() {
        let mut packet = Vec::from(*proto::MAGIC);
        // version, an unknown mode, and a zero-length body.
        packet.extend_from_slice(&[1, 200, 0, 0]);
        let mut errbuf = Vec::new();

        let payload = interpret_packet(&packet, 200, 0, &mut errbuf);

        assert!(payload.is_none(), "an unknown mode cannot be decoded");
        assert_eq!(errbuf.len(), 1, "and it must leave a reason behind");
        assert!(
            errbuf[0].contains("mode 200") && errbuf[0].contains("host is reachable"),
            "the reason must name the mode and clear the host: {:?}",
            errbuf[0]
        );
    }

    /// The reason goes in the same bounded buffer as the stderr lines, so a
    /// stream that produces nothing but unreadable packets cannot grow it
    /// without limit.
    #[test]
    fn the_reason_respects_the_buffer_bound() {
        let mut packet = Vec::from(*proto::MAGIC);
        packet.extend_from_slice(&[1, 200, 0, 0]);
        let mut errbuf = Vec::new();
        for _ in 0..(super::MAX_STDERR_LINES * 3) {
            interpret_packet(&packet, 200, 0, &mut errbuf);
        }
        assert_eq!(errbuf.len(), super::MAX_STDERR_LINES);
    }

    /// A remote that printed a banner instead of running the agent must be
    /// reported as exactly that.
    ///
    /// The three readers of this stream all pattern-match
    /// `while let Ok(Some(payload))`, so the `Err` this returns ends their loop
    /// and is dropped. What they report is built from `errbuf`: the monitor
    /// takes its last line and otherwise says `Connection to <host> closed`,
    /// and fetch and docker say only what is in it. So the reason has to be in
    /// `errbuf`, not only in the error.
    #[test]
    fn a_banner_where_the_framing_should_be_is_not_reported_as_a_closed_connection() {
        let err = super::framing_lost(b"Welcome ", Some("Welcome to Ubuntu 24.04".to_string()));

        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
        let reason = super::describe_failure(&err);
        assert!(
            reason.contains("Welcome to Ubuntu 24.04"),
            "the reason must quote what the remote actually said: {reason:?}"
        );
        assert!(
            reason.contains("login banner"),
            "and name the cause the operator can act on: {reason:?}"
        );
    }

    /// A desync with no banner behind it still says the host is reachable,
    /// rather than leaving the panel to claim it went away.
    #[test]
    fn a_mid_stream_desync_still_clears_the_host() {
        let err = super::framing_lost(&[0xff; 8], None);
        assert!(
            super::describe_failure(&err).contains("host is reachable"),
            "{err}"
        );
    }

    /// The three siblings the framing fix left behind: the two short header
    /// reads and the payload read all return a plain I/O error, and every one of
    /// them was silent. Fixing only the framing case would have left an I/O
    /// error mid-stream stopping the monitor with `Connection to <host> closed`
    /// and stopping fetch and docker with nothing at all.
    #[test]
    fn a_plain_io_failure_is_described_rather_than_left_bare() {
        for kind in [
            std::io::ErrorKind::BrokenPipe,
            std::io::ErrorKind::ConnectionReset,
            std::io::ErrorKind::TimedOut,
        ] {
            let reason = super::describe_failure(&std::io::Error::from(kind));
            assert!(
                reason.contains("reading from the agent failed"),
                "{kind:?} must arrive as something an operator can read: {reason:?}"
            );
            assert!(
                reason.contains("may still be reachable"),
                "{kind:?} must not read as the host going away: {reason:?}"
            );
        }
    }

    /// Both reason paths and the stderr path share one bound. They used to be
    /// written twice with two different rules -- `>` and `>=` -- so the buffer
    /// held nine lines or eight depending on which kind of line arrived last.
    #[test]
    fn every_path_into_the_buffer_respects_one_bound() {
        let mut errbuf = Vec::new();
        let framing = super::framing_lost(&[0xff; 8], None);
        for i in 0..(super::MAX_STDERR_LINES * 2) {
            super::note(&mut errbuf, format!("stderr {i}"));
            super::note(&mut errbuf, super::describe_failure(&framing));
        }
        assert_eq!(errbuf.len(), super::MAX_STDERR_LINES);
    }

    /// A packet that decodes is handed back untouched, with nothing added to
    /// the buffer that reports failures.
    #[test]
    fn a_readable_packet_is_returned_and_reports_nothing() {
        let payload = proto::Payload::Fetch(multitop_agent::fetch::FetchSnapshot::default());
        let encoded = proto::encode_packet(&payload);
        let len = encoded.len() - 8;
        let mut errbuf = Vec::new();

        let got = interpret_packet(&encoded, encoded[5], len, &mut errbuf);

        assert!(got.is_some(), "a well-formed packet must decode");
        assert!(errbuf.is_empty(), "and must report nothing: {errbuf:?}");
    }
}

#[cfg(test)]
mod handshake_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::{read_handshake, Handshake};
    use tokio::io::{AsyncRead, ReadBuf};

    /// A reader that hands back one byte at a time, which a pipe is entitled to
    /// do and which the old four-bytes-in-one-`read` handshake could not
    /// survive.
    struct Dribble {
        bytes: Vec<u8>,
        at: usize,
    }

    impl AsyncRead for Dribble {
        fn poll_read(
            mut self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
            buf: &mut ReadBuf<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            if self.at < self.bytes.len() && buf.remaining() > 0 {
                let b = self.bytes[self.at];
                self.at += 1;
                buf.put_slice(&[b]);
            }
            std::task::Poll::Ready(Ok(()))
        }
    }

    fn dribbled(bytes: &[u8]) -> tokio::io::BufReader<Dribble> {
        // Capacity 1, so the `BufReader` cannot paper over the split the way a
        // default-sized one would.
        tokio::io::BufReader::with_capacity(
            1,
            Dribble {
                bytes: bytes.to_vec(),
                at: 0,
            },
        )
    }

    /// The regression. A framed stream whose magic arrives a byte at a time is
    /// still a framed stream; read four bytes at a time with `read`, it was
    /// mistaken for text, and every packet after it read from the wrong offset.
    #[tokio::test]
    async fn a_magic_header_split_across_reads_is_still_recognised() {
        let mut framed = Vec::from(*multitop_agent::proto::MAGIC);
        framed.extend_from_slice(&[0, 0, 4, 0]);
        let mut reader = dribbled(&framed);
        assert_eq!(read_handshake(&mut reader).await, Handshake::Framed);
    }

    #[tokio::test]
    async fn a_need_agent_line_names_its_architecture() {
        let mut reader = dribbled(b"===NEEDAGENT=== aarch64\n");
        assert_eq!(
            read_handshake(&mut reader).await,
            Handshake::NeedAgent("aarch64".to_string())
        );
    }

    /// A banner is left for the packet reader to fail on -- and it must be
    /// carried, not dropped. Dropped, the reader fails on the *next* eight
    /// bytes with `invalid magic header` and the panel can only say the
    /// connection closed, about a host that is up and printing a banner.
    #[tokio::test]
    async fn a_banner_is_carried_so_the_failure_can_name_it() {
        let mut reader = dribbled(b"Welcome to Ubuntu 24.04\n");
        assert_eq!(
            read_handshake(&mut reader).await,
            Handshake::Text("Welcome to Ubuntu 24.04".to_string()),
            "the text that made this not-framed has to survive the handshake"
        );
    }

    /// Fewer than four bytes and then nothing is a closed connection, not a
    /// three-byte banner to go hunting through.
    #[tokio::test]
    async fn a_stream_that_closes_early_is_closed() {
        assert_eq!(read_handshake(&mut dribbled(b"")).await, Handshake::Closed);
        assert_eq!(
            read_handshake(&mut dribbled(b"hi")).await,
            Handshake::Closed
        );
    }
}
