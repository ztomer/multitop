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
}

/// What the first bytes of a session turned out to be.
#[derive(Debug, PartialEq, Eq)]
pub enum Handshake {
    /// The agent is running and the stream is already framed.
    Framed,
    /// The remote has no usable agent for the architecture it named.
    NeedAgent(String),
    /// Something else came out first -- a banner, an error. Left to the packet
    /// reader to fail on, so the text lands in the panel.
    Text,
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
    ssh::parse_need_agent(&line).map_or(Handshake::Text, |arch| {
        Handshake::NeedAgent(arch.to_string())
    })
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
            .map_err(|e| match e.kind() {
                std::io::ErrorKind::NotFound => "ssh command not found".to_string(),
                _ => format!("ssh: {e}"),
            })?;

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
                })
            }
            Handshake::Text => {
                return Ok(PacketStream {
                    child,
                    stdout,
                    stderr,
                    pending_header: None,
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

/// Read next packet from stream.
///
/// # Errors
///
/// Returns an error if reading from stdout/stderr fails or process exits.
pub async fn next_packet(
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
                        errbuf.push(line);
                        if errbuf.len() > MAX_STDERR_LINES {
                            errbuf.remove(0);
                        }
                    }
                }
            }
        }
    }

    if &header[..4] != proto::MAGIC {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "invalid magic header",
        ));
    }
    let len = u16::from_le_bytes([header[6], header[7]]) as usize;
    let mut payload_bytes = vec![0u8; len];
    stream.stdout.read_exact(&mut payload_bytes).await?;

    let mut full_packet = Vec::with_capacity(8 + len);
    full_packet.extend_from_slice(&header);
    full_packet.extend_from_slice(&payload_bytes);

    Ok(proto::decode_packet(&full_packet))
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

    #[tokio::test]
    async fn a_banner_is_left_for_the_packet_reader_to_fail_on() {
        let mut reader = dribbled(b"Welcome to Ubuntu 24.04\n");
        assert_eq!(read_handshake(&mut reader).await, Handshake::Text);
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
