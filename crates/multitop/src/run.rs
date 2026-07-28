//! The async runtime: terminal event loop plus one SSH task per panel.

use std::time::Duration;

use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use tokio::io::{BufReader, Lines};
use tokio::process::{Child, ChildStderr, ChildStdout};
use tokio::sync::mpsc::{self, Sender};
use tokio::task::JoinHandle;
use tokio::time::{sleep, Instant};
use tokio_stream::StreamExt as _;

use crate::app::{error_line, status_line, App, Command, Msg};
use crate::config::Server;
use crate::ssh::{self, Arch, Mode};
use crate::ui;

/// How long to wait after the last resize event before restarting the agents
/// at the new size. Dragging a window edge emits a burst of events.
const RESIZE_DEBOUNCE: Duration = Duration::from_millis(30);

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

use multitop_agent::SortBy;

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
        tasks.monitors[i] = Some(spawn_monitor(i, server.clone(), dims, app.sort, tx.clone()));
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
                    // With binary telemetry, resizes happen 100% locally in Ratatui
                    // without restarting SSH tasks!
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
    match key.code {
        KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('Q') => {
            app.quit();
            return;
        }
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.quit();
            return;
        }
        KeyCode::Char('c') | KeyCode::Char('C') => {
            let old_sort = app.sort;
            app.sort = SortBy::Cpu;
            if old_sort != app.sort {
                restart_all_agents(app, servers, dims, tx, tasks);
            }
            return;
        }
        KeyCode::Char('m') | KeyCode::Char('M') => {
            let old_sort = app.sort;
            app.sort = SortBy::Mem;
            if old_sort != app.sort {
                restart_all_agents(app, servers, dims, tx, tasks);
            }
            return;
        }
        _ => {}
    }

    let cmds = match key.code {
        KeyCode::Char('d') | KeyCode::Char('D') => app.toggle_docker(),
        KeyCode::Char('s') | KeyCode::Char('S') => app.switch_stats(),
        KeyCode::Char('u') | KeyCode::Char('U') => app.run_upgrade(),
        _ => return,
    };

    for cmd in cmds {
        let (idx, handle) = match cmd {
            Command::RunDocker { panel, gen } => (
                panel,
                spawn_docker(panel, gen, servers[panel].clone(), dims, app.sort, tx.clone()),
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

fn restart_all_agents(
    app: &App,
    servers: &[Server],
    dims: (u16, u16),
    tx: &Sender<Msg>,
    tasks: &mut Tasks,
) {
    for (i, server) in servers.iter().enumerate() {
        if let Some(h) = tasks.monitors[i].take() {
            h.abort();
        }
        tasks.monitors[i] = Some(spawn_monitor(i, server.clone(), dims, app.sort, tx.clone()));
    }
    if app.in_docker() {
        for (i, panel) in app.panels.iter().enumerate() {
            if panel.mode == crate::app::Mode::Docker {
                let gen = panel.gen;
                if let Some(old) = tasks.aux[i].replace(spawn_docker(i, gen, servers[i].clone(), dims, app.sort, tx.clone())) {
                    old.abort();
                }
            }
        }
    }
}

// ------------------------------------------------------------------- streams

pub(crate) struct PacketStream {
    pub _child: Child,
    pub stdout: BufReader<ChildStdout>,
    pub stderr: Lines<BufReader<ChildStderr>>,
    pub pending_header: Option<[u8; 4]>,
}

/// Start the remote agent, uploading it first if the host has no cached copy.
pub(crate) async fn connect(
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

pub(crate) async fn next_packet(
    stream: &mut PacketStream,
    errbuf: &mut Vec<String>,
) -> std::io::Result<Option<multitop_agent::proto::Payload>> {
    use tokio::io::AsyncReadExt;
    use multitop_agent::proto;

    let mut header = [0u8; 8];
    if let Some(pending4) = stream.pending_header.take() {
        header[..4].copy_from_slice(&pending4);
        if let Err(e) = stream.stdout.read_exact(&mut header[4..8]).await {
            return Err(e);
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

// --------------------------------------------------------------------- tasks

/// Long-lived: streams monitor frames and reconnects on failure.
///
/// This task keeps running through Docker and Upgrade views, so stats stay
/// warm and switching back is instant.
fn spawn_monitor(
    idx: usize,
    server: Server,
    dims: (u16, u16),
    sort: SortBy,
    tx: Sender<Msg>,
) -> JoinHandle<()> {
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

            match connect(&server, Mode::Monitor, sort, notify).await {
                Ok(mut stream) => {
                    failures = 0;
                    let mut errbuf = Vec::new();
                    let pal = &multitop_agent::color::ANSI;

                    while let Ok(Some(payload)) = next_packet(&mut stream, &mut errbuf).await {
                        let lines = match payload {
                            multitop_agent::proto::Payload::Monitor(snap) => {
                                multitop_agent::render::render(
                                    &snap,
                                    dims.0 as usize,
                                    dims.1 as usize,
                                    multitop_agent::render::bar_len_for(dims.0 as usize),
                                    pal,
                                )
                            }
                            multitop_agent::proto::Payload::Docker { host, rows } => {
                                multitop_agent::docker::render(
                                    &host,
                                    dims.0 as usize,
                                    dims.1 as usize,
                                    &rows,
                                    pal,
                                    sort,
                                )
                            }
                        };
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

fn spawn_docker(
    idx: usize,
    gen: u64,
    server: Server,
    dims: (u16, u16),
    sort: SortBy,
    tx: Sender<Msg>,
) -> JoinHandle<()> {
    crate::tasks::spawn_docker(idx, gen, server, dims, sort, tx)
}

fn spawn_upgrade(idx: usize, gen: u64, server: Server, tx: Sender<Msg>) -> JoinHandle<()> {
    crate::tasks::spawn_upgrade(idx, gen, server, tx)
}
