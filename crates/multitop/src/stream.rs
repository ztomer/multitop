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
    #[allow(clippy::pub_underscore_fields)]
    pub _child: Child,
    pub stdout: BufReader<ChildStdout>,
    pub stderr: Lines<BufReader<ChildStderr>>,
    pub pending_header: Option<[u8; 4]>,
}

#[allow(clippy::missing_panics_doc, clippy::missing_errors_doc, clippy::expect_used)]
pub async fn connect(
    server: &Server,
    mode: Mode,
    sort: SortBy,
    on_status: impl Fn(String),
) -> Result<PacketStream, String> {
    use tokio::io::AsyncBufReadExt;
    use tokio::io::AsyncReadExt;

    for attempt in 0..2 {
        let mut child = ssh::spawn_agent(server, mode, sort)
            .await
            .map_err(|e| match e.kind() {
                std::io::ErrorKind::NotFound => "ssh command not found".to_string(),
                _ => format!("ssh: {e}"),
            })?;

        let stdout = child.stdout.take().expect("stdout piped");
        let stderr = child.stderr.take().expect("stderr piped");
        let mut stdout = BufReader::new(stdout);
        let mut stderr = BufReader::new(stderr).lines();

        let mut first4 = [0u8; 4];
        let n = stdout.read(&mut first4).await.unwrap_or(0);
        if n == 0 {
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

        if n >= 4 && &first4 == multitop_agent::proto::MAGIC {
            return Ok(PacketStream {
                _child: child,
                stdout,
                stderr,
                pending_header: Some(first4),
            });
        }

        let mut line_buf = String::from_utf8_lossy(&first4[..n]).to_string();
        let mut rest_line = String::new();
        let _ = stdout.read_line(&mut rest_line).await;
        line_buf.push_str(&rest_line);

        let Some(arch_str) = ssh::parse_need_agent(&line_buf) else {
            return Ok(PacketStream {
                _child: child,
                stdout,
                stderr,
                pending_header: None,
            });
        };

        if attempt > 0 {
            return Err(format!(
                "Agent did not start on {} after install",
                server.host
            ));
        }
        let Some(arch) = Arch::from_uname(arch_str) else {
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

#[allow(clippy::missing_errors_doc)]
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
