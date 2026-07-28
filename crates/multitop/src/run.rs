//! The async runtime: terminal event loop plus one SSH task per panel.

use std::time::Duration;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use tokio::io::{AsyncBufReadExt, BufReader, Lines};
use tokio::process::{Child, ChildStderr, ChildStdout};
use tokio::sync::mpsc::{self, Sender};
use tokio::task::JoinHandle;
use tokio::time::{sleep, Instant};
use tokio_stream::StreamExt as _;

use crate::app::{error_line, header_line, status_line, App, Command, Msg};
use crate::config::Server;
use crate::ssh::{self, Arch, Mode};
use crate::ui;

/// Frame delimiter the agent writes between refreshes.
const FRAME_MARKER: &str = "===MONITOR===";

/// How long to wait after the last resize event before restarting the agents
/// at the new size. Dragging a window edge emits a burst of events; without
/// this every intermediate width would tear down and rebuild every SSH task.
const RESIZE_DEBOUNCE: Duration = Duration::from_millis(250);

/// Backoff between reconnection attempts after a dropped SSH session.
const RECONNECT_BACKOFF: [u64; 4] = [2, 5, 10, 20];

/// Stderr retained for the failure message when a connection dies.
const MAX_STDERR_LINES: usize = 8;

pub async fn run(servers: Vec<Server>) -> std::io::Result<()> {
    let mut terminal = ratatui::init();
    let result = event_loop(&mut terminal, servers).await;
    ratatui::restore();
    result
}

struct Tasks {
    monitors: Vec<Option<JoinHandle<()>>>,
    aux: Vec<Option<JoinHandle<()>>>,
}

impl Tasks {
    fn new(n: usize) -> Self {
        Tasks {
            monitors: (0..n).map(|_| None).collect(),
            aux: (0..n).map(|_| None).collect(),
        }
    }

    /// Aborting a task drops the `Child` it owns, and every child is spawned
    /// with `kill_on_drop`, so this also terminates the SSH process.
    fn abort_all(&mut self) {
        for h in self
            .monitors
            .iter_mut()
            .chain(self.aux.iter_mut())
            .flatten()
        {
            h.abort();
        }
    }
}

async fn event_loop(
    terminal: &mut ratatui::DefaultTerminal,
    servers: Vec<Server>,
) -> std::io::Result<()> {
    let n = servers.len();
    let mut app = App::new(servers.clone());
    let (tx, mut rx) = mpsc::channel::<Msg>(512);
    let mut tasks = Tasks::new(n);
    let mut events = crossterm::event::EventStream::new();

    let mut dims = ui::agent_dims(terminal.size()?, n);
    for (i, server) in servers.iter().enumerate() {
        tasks.monitors[i] = Some(spawn_monitor(i, server.clone(), dims, tx.clone()));
    }

    let mut resize_at: Option<Instant> = None;
    let mut dirty = true;

    loop {
        if dirty {
            terminal.draw(|f| ui::draw(f, &app))?;
            dirty = false;
        }

        let resize_wait = async {
            match resize_at {
                Some(at) => tokio::time::sleep_until(at).await,
                None => std::future::pending().await,
            }
        };

        tokio::select! {
            biased;

            maybe = events.next() => {
                match maybe {
                    Some(Ok(Event::Key(key))) => {
                        handle_key(key, &mut app, &servers, dims, &tx, &mut tasks);
                        dirty = true;
                    }
                    Some(Ok(Event::Resize(..))) => {
                        resize_at = Some(Instant::now() + RESIZE_DEBOUNCE);
                        dirty = true;
                    }
                    Some(Ok(_)) => {}
                    // The terminal went away; leaving would strand the SSH
                    // children, so exit through the normal path.
                    Some(Err(_)) | None => app.quit(),
                }
            }

            Some(msg) = rx.recv() => {
                app.apply(msg);
                // A burst of frames should cost one draw, not one each.
                while let Ok(msg) = rx.try_recv() {
                    app.apply(msg);
                }
                dirty = true;
            }

            _ = resize_wait, if resize_at.is_some() => {
                resize_at = None;
                let new_dims = ui::agent_dims(terminal.size()?, n);
                if new_dims != dims {
                    dims = new_dims;
                    // The agent formats to a fixed width, so a resize means
                    // restarting it rather than reflowing what it already sent.
                    for (i, server) in servers.iter().enumerate() {
                        if let Some(h) = tasks.monitors[i].take() {
                            h.abort();
                        }
                        tasks.monitors[i] = Some(spawn_monitor(i, server.clone(), dims, tx.clone()));
                    }
                }
                dirty = true;
            }
        }

        if app.should_quit {
            tasks.abort_all();
            return Ok(());
        }
    }
}

fn handle_key(
    key: KeyEvent,
    app: &mut App,
    servers: &[Server],
    dims: (u16, u16),
    tx: &Sender<Msg>,
    tasks: &mut Tasks,
) {
    // Key *releases* also arrive on terminals that report them; acting on
    // both would run every action twice.
    if key.kind != KeyEventKind::Press {
        return;
    }
    let cmds = match key.code {
        KeyCode::Esc => {
            app.quit();
            return;
        }
        KeyCode::Char('q') => {
            app.quit();
            return;
        }
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.quit();
            return;
        }
        KeyCode::Char('d') | KeyCode::Char('D') => app.toggle_docker(),
        KeyCode::Char('s') | KeyCode::Char('S') => app.switch_stats(),
        KeyCode::Char('u') | KeyCode::Char('U') => app.run_upgrade(),
        _ => return,
    };

    for cmd in cmds {
        let (idx, handle) = match cmd {
            Command::RunDocker { panel, gen } => (
                panel,
                spawn_docker(panel, gen, servers[panel].clone(), dims, tx.clone()),
            ),
            Command::RunUpgrade { panel, gen } => (
                panel,
                spawn_upgrade(panel, gen, servers[panel].clone(), tx.clone()),
            ),
        };
        // Supersede whatever that panel was running.
        if let Some(old) = tasks.aux[idx].replace(handle) {
            old.abort();
        }
    }
}

// ------------------------------------------------------------------- streams

struct Stream {
    _child: Child,
    stdout: Lines<BufReader<ChildStdout>>,
    stderr: Lines<BufReader<ChildStderr>>,
    /// The line already consumed while checking for the NEEDAGENT marker.
    pending: Option<String>,
}

/// Start the remote agent, uploading it first if the host has no cached copy.
async fn connect(
    server: &Server,
    mode: Mode,
    dims: (u16, u16),
    on_status: impl Fn(String),
) -> Result<Stream, String> {
    for attempt in 0..2 {
        let mut child = ssh::spawn_agent(server, mode, dims.0, dims.1)
            .await
            .map_err(|e| match e.kind() {
                std::io::ErrorKind::NotFound => "ssh command not found".to_string(),
                _ => format!("ssh: {e}"),
            })?;

        let stdout = child.stdout.take().expect("stdout piped");
        let stderr = child.stderr.take().expect("stderr piped");
        let mut stdout = BufReader::new(stdout).lines();
        let mut stderr = BufReader::new(stderr).lines();

        let first = stdout.next_line().await.map_err(|e| format!("read: {e}"))?;

        let Some(line) = first else {
            // No stdout at all: the failure reason is on stderr.
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
        };

        let Some(arch_str) = ssh::parse_need_agent(&line) else {
            return Ok(Stream {
                _child: child,
                stdout,
                stderr,
                pending: Some(line),
            });
        };

        // The agent is missing from the host's cache.
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

/// Read the next stdout line, remembering stderr as it arrives.
async fn next_line(
    stream: &mut Stream,
    errbuf: &mut Vec<String>,
) -> std::io::Result<Option<String>> {
    if let Some(line) = stream.pending.take() {
        return Ok(Some(line));
    }
    loop {
        tokio::select! {
            line = stream.stdout.next_line() => return line,
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

// --------------------------------------------------------------------- tasks

/// Long-lived: streams monitor frames and reconnects on failure.
///
/// This task keeps running through Docker and Upgrade views, so stats stay
/// warm and switching back is instant.
fn spawn_monitor(idx: usize, server: Server, dims: (u16, u16), tx: Sender<Msg>) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut failures = 0usize;
        loop {
            // Status messages from a monitor restart are only worth showing
            // while the panel has nothing better; generation 0 is never
            // superseded for the monitor stream, so send them unconditionally
            // as frames of one line.
            let status_tx = tx.clone();
            let notify = move |text: String| {
                let _ = status_tx.try_send(Msg::Frame {
                    panel: idx,
                    lines: vec![text],
                });
            };

            match connect(&server, Mode::Monitor, dims, notify).await {
                Ok(mut stream) => {
                    failures = 0;
                    let mut errbuf = Vec::new();
                    let mut frame: Vec<String> = Vec::new();

                    // Ends on EOF or a read error; both fall through to the
                    // reconnect below.
                    while let Ok(Some(line)) = next_line(&mut stream, &mut errbuf).await {
                        if line.trim_end() != FRAME_MARKER {
                            frame.push(line);
                            continue;
                        }
                        if frame.is_empty() {
                            continue;
                        }
                        let lines = std::mem::take(&mut frame);
                        if tx.send(Msg::Frame { panel: idx, lines }).await.is_err() {
                            return;
                        }
                    }

                    let detail = errbuf
                        .last()
                        .cloned()
                        .unwrap_or_else(|| format!("Connection to {} closed", server.host));
                    let _ = tx
                        .send(Msg::Frame {
                            panel: idx,
                            lines: vec![error_line(detail)],
                        })
                        .await;
                }
                Err(e) => {
                    let _ = tx
                        .send(Msg::Frame {
                            panel: idx,
                            lines: vec![error_line(e)],
                        })
                        .await;
                }
            }

            let wait = RECONNECT_BACKOFF[failures.min(RECONNECT_BACKOFF.len() - 1)];
            failures += 1;
            sleep(Duration::from_secs(wait)).await;
        }
    })
}

/// One-shot: renders the Docker view for a panel.
fn spawn_docker(
    idx: usize,
    gen: u64,
    server: Server,
    dims: (u16, u16),
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

        let mut stream = match connect(&server, Mode::Docker, dims, notify).await {
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
        while let Ok(Some(line)) = next_line(&mut stream, &mut errbuf).await {
            if tx
                .send(Msg::AuxLine {
                    panel: idx,
                    gen,
                    line,
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

/// One-shot: runs the server's `upgrade_cmd`, streaming its output.
fn spawn_upgrade(idx: usize, gen: u64, server: Server, tx: Sender<Msg>) -> JoinHandle<()> {
    tokio::spawn(async move {
        let Some(command) = server.upgrade_cmd.clone() else {
            return;
        };

        let mut child = match ssh::spawn_command(&server, &command) {
            Ok(c) => c,
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
        let stdout = child.stdout.take().expect("stdout piped");
        let stderr = child.stderr.take().expect("stderr piped");
        let mut stream = Stream {
            _child: child,
            stdout: BufReader::new(stdout).lines(),
            stderr: BufReader::new(stderr).lines(),
            pending: None,
        };

        let header = header_line(format!("Upgrade on {}", server.host));
        let _ = tx
            .send(Msg::AuxBegin {
                panel: idx,
                gen,
                header: Some(header),
            })
            .await;

        let mut errbuf = Vec::new();
        while let Ok(Some(line)) = next_line(&mut stream, &mut errbuf).await {
            if tx
                .send(Msg::AuxLine {
                    panel: idx,
                    gen,
                    line,
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
        let _ = tx
            .send(Msg::AuxDone {
                panel: idx,
                gen,
                note: Some(status_line("\u{2500} done")),
            })
            .await;
    })
}
