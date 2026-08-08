use crate::app::Msg;
use crate::config::Server;
use crate::fmt::error_line;
use crate::ssh;
use crate::stream::{connect, next_packet};
use multitop_agent::SortBy;
use tokio::io::AsyncWriteExt;
use tokio::io::BufReader;
use tokio::sync::mpsc::Sender;
use tokio::task::JoinHandle;

pub const MAX_SENTINEL_LINES: usize = 50;

/// How long to wait for that sentinel before giving up.
///
/// Generous enough for a slow link and a chatty login banner, short enough that
/// a wedged remote costs one message rather than the rest of the session. The
/// line count alone is not a bound: a remote that prints nothing makes it
/// unreachable.
pub const SENTINEL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

#[must_use]
#[allow(
    clippy::missing_panics_doc,
    clippy::too_many_lines,
    clippy::expect_used
)]
pub async fn deliver_sudo_password(
    stdout_lines: &mut tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
    stdin: Option<tokio::process::ChildStdin>,
    secret: &str,
    patience: std::time::Duration,
) -> bool {
    let hunt = async {
        for _ in 0..MAX_SENTINEL_LINES {
            match stdout_lines.next_line().await {
                Ok(Some(line)) if line.trim() == ssh::PW_READY_SENTINEL => return true,
                // Anything else this early is banner noise; keep looking.
                Ok(Some(_)) => {}
                _ => return false,
            }
        }
        false
    };
    let ready = tokio::time::timeout(patience, hunt).await.unwrap_or(false);
    if let Some(mut stdin) = stdin {
        if ready {
            let _ = stdin.write_all(format!("{secret}\n").as_bytes()).await;
            let _ = stdin.flush().await;
        }
        // Dropped here on both paths, so the remote `read` cannot block.
    }
    ready
}

#[must_use]
pub fn spawn_fetch(
    idx: usize,
    gen: u64,
    epoch: u64,
    server: Server,
    dims: (u16, u16),
    sort: SortBy,
    tx: Sender<Msg>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let status_tx = tx.clone();
        let notify = move |text: String| {
            let _ = status_tx.try_send(Msg::Status {
                panel: idx,
                gen,
                text,
            });
        };

        let mut stream = match connect(&server, ssh::Mode::Fetch, sort, notify).await {
            Ok(s) => s,
            Err(e) => {
                let _ = tx
                    .send(Msg::Status {
                        panel: idx,
                        gen,
                        text: error_line(e),
                    })
                    .await;
                return;
            }
        };

        let mut errbuf = Vec::new();

        while let Ok(Some(payload)) = next_packet(&mut stream, &mut errbuf).await {
            if tx
                .send(Msg::Packet {
                    panel: idx,
                    gen,
                    epoch,
                    payload,
                    dims,
                })
                .await
                .is_err()
            {
                return;
            }
        }
        for line in errbuf {
            let _ = tx
                .send(Msg::AuxLine {
                    panel: idx,
                    gen,
                    line: error_line(line),
                })
                .await;
        }
    })
}

/// One-shot: renders the Docker view for a panel.
#[must_use]
pub fn spawn_docker(
    idx: usize,
    gen: u64,
    epoch: u64,
    server: Server,
    dims: (u16, u16),
    sort: SortBy,
    tx: Sender<Msg>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let status_tx = tx.clone();
        let notify = move |text: String| {
            let _ = status_tx.try_send(Msg::Status {
                panel: idx,
                gen,
                text,
            });
        };

        let mut stream = match connect(&server, ssh::Mode::Docker, sort, notify).await {
            Ok(s) => s,
            Err(e) => {
                let _ = tx
                    .send(Msg::Status {
                        panel: idx,
                        gen,
                        text: error_line(e),
                    })
                    .await;
                return;
            }
        };

        let _ = tx
            .send(Msg::AuxBegin {
                panel: idx,
                gen,
                header: None,
            })
            .await;
        let mut errbuf = Vec::new();

        while let Ok(Some(payload)) = next_packet(&mut stream, &mut errbuf).await {
            if tx
                .send(Msg::Packet {
                    panel: idx,
                    gen,
                    epoch,
                    payload,
                    dims,
                })
                .await
                .is_err()
            {
                return;
            }
        }
        for line in errbuf {
            let _ = tx
                .send(Msg::AuxLine {
                    panel: idx,
                    gen,
                    line: error_line(line),
                })
                .await;
        }
    })
}
