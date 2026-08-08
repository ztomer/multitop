use crate::config::Server;
use crate::fmt::status_line;
use crate::ssh::{self, Arch, Mode};
use multitop_agent::SortBy;
use tokio::io::{BufReader, Lines};
use tokio::process::{Child, ChildStderr, ChildStdout};

//  SSH stream protocol: packet framing, agent bootstrap, and connection.

/// Max stderr lines retained for the failure message when a connection dies.
pub const MAX_STDERR_LINES: usize = 8;

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
pub fn spawn_failure(server: &Server, e: &std::io::Error) -> String {
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
pub fn describe_failure(e: &std::io::Error) -> String {
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
pub fn note(errbuf: &mut Vec<String>, reason: String) {
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
pub fn framing_lost(header: &[u8], preamble: Option<String>) -> std::io::Error {
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
pub fn interpret_packet(
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
