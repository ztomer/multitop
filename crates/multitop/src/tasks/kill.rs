//! One-shot `kill -9 <pid>` via the Exec pty.
//!
//! Reuses the same `MTOP` framing as upgrade, but with `use_lock:false` and
//! no `STARTED` state: a kill is a momentary action, not a session-long log.
//! Mirrors `tasks/upgrade.rs` for agent install retry and stall handling.

use tokio::sync::mpsc::Sender;
use tokio::task::JoinHandle;

use crate::app::Msg;
use crate::config::Server;

use super::exec_runner::{generic_exec, ExecAction};

#[must_use]
pub fn spawn_kill(
    idx: usize,
    gen: u64,
    server: Server,
    pid: u32,
    name: String,
    pass: Option<String>,
    tx: Sender<Msg>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let outcome = run_kill(idx, gen, &server, pid, &name, pass.as_deref(), &tx).await;
        let _ = tx
            .send(Msg::AuxDone {
                panel: idx,
                gen,
                note: Some(outcome),
                success: false,
            })
            .await;
    })
}

#[must_use]
pub fn spawn_journal(
    idx: usize,
    gen: u64,
    server: Server,
    pid: u32,
    name: String,
    pass: Option<String>,
    tx: Sender<Msg>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let outcome = run_journal(idx, gen, &server, pid, &name, pass.as_deref(), &tx).await;
        let _ = tx
            .send(Msg::AuxDone {
                panel: idx,
                gen,
                note: Some(outcome),
                success: false,
            })
            .await;
    })
}

#[must_use]
pub fn spawn_renice(
    idx: usize,
    gen: u64,
    server: Server,
    pid: u32,
    name: String,
    pass: Option<String>,
    tx: Sender<Msg>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let outcome = run_renice(idx, gen, &server, pid, &name, pass.as_deref(), &tx).await;
        let _ = tx
            .send(Msg::AuxDone {
                panel: idx,
                gen,
                note: Some(outcome),
                success: false,
            })
            .await;
    })
}

#[must_use]
pub fn spawn_tail(
    idx: usize,
    gen: u64,
    server: Server,
    pass: Option<String>,
    tx: Sender<Msg>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let outcome = run_tail(idx, gen, &server, pass.as_deref(), &tx).await;
        let _ = tx
            .send(Msg::AuxDone {
                panel: idx,
                gen,
                note: Some(outcome),
                success: false,
            })
            .await;
    })
}

/// Refresh interval for custom exec panels per roadmap Phase 3.
const CUSTOM_PANEL_POLL_INTERVAL_MS: u64 = 250;

/// `[[panels]] command="nvidia-smi …"` — runs every 250 ms via Exec pty,
/// rendered as a Fetch card. Reuses the same `MTOP` framing as kill/tail.
#[must_use]
pub fn spawn_custom(
    idx: usize,
    gen: u64,
    server: Server,
    command: String,
    pass: Option<String>,
    tx: Sender<Msg>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_millis(
            CUSTOM_PANEL_POLL_INTERVAL_MS,
        ));
        loop {
            interval.tick().await;
            let _ = run_custom_once(idx, gen, &server, &command, pass.as_deref(), &tx).await;
        }
    })
}

async fn run_custom_once(
    idx: usize,
    gen: u64,
    server: &Server,
    command: &str,
    pass: Option<&str>,
    tx: &Sender<Msg>,
) -> String {
    let header = format!("{command} on {}", server.host);
    generic_exec(&ExecAction {
        idx,
        gen,
        server,
        command,
        pass,
        tx,
        header: &header,
        action_desc: "custom",
    })
    .await
}

async fn run_kill(
    idx: usize,
    gen: u64,
    server: &Server,
    pid: u32,
    name: &str,
    pass: Option<&str>,
    tx: &Sender<Msg>,
) -> String {
    let command = format!("kill -9 {pid}");
    let header = format!("Kill {}:{pid}:{name} on {}", server.host, server.host);
    let desc = format!("kill {pid}:{name}");
    generic_exec(&ExecAction {
        idx,
        gen,
        server,
        command: &command,
        pass,
        tx,
        header: &header,
        action_desc: &desc,
    })
    .await
}

async fn run_journal(
    idx: usize,
    gen: u64,
    server: &Server,
    pid: u32,
    name: &str,
    pass: Option<&str>,
    tx: &Sender<Msg>,
) -> String {
    let command = format!(
        "journalctl --no-pager -n 200 -f -u {name}.service 2>/dev/null || journalctl --no-pager -n 200 -f --pid={pid} 2>/dev/null || tail -F /proc/{pid}/fd/1 2>/dev/null || tail -n 200 -F /var/log/syslog"
    );
    let header = format!("Journal {}:{pid}:{name} on {}", server.host, server.host);
    let desc = format!("journal {pid}:{name}");
    generic_exec(&ExecAction {
        idx,
        gen,
        server,
        command: &command,
        pass,
        tx,
        header: &header,
        action_desc: &desc,
    })
    .await
}

async fn run_renice(
    idx: usize,
    gen: u64,
    server: &Server,
    pid: u32,
    name: &str,
    pass: Option<&str>,
    tx: &Sender<Msg>,
) -> String {
    let command = format!("renice -n 10 -p {pid}");
    let header = format!("Renice {}:{pid}:{name} on {}", server.host, server.host);
    let desc = format!("renice {pid}:{name}");
    generic_exec(&ExecAction {
        idx,
        gen,
        server,
        command: &command,
        pass,
        tx,
        header: &header,
        action_desc: &desc,
    })
    .await
}

async fn run_tail(
    idx: usize,
    gen: u64,
    server: &Server,
    pass: Option<&str>,
    tx: &Sender<Msg>,
) -> String {
    let command = "tail -n 200 -F /var/log/syslog 2>/dev/null || tail -n 200 -F /var/log/messages 2>/dev/null || journalctl --no-pager -n 200 -f 2>/dev/null";
    let header = format!("Tail syslog on {}", server.host);
    generic_exec(&ExecAction {
        idx,
        gen,
        server,
        command,
        pass,
        tx,
        header: &header,
        action_desc: "tail",
    })
    .await
}
