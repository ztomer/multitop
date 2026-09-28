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

/// What a process action needs to know, as one value.
///
/// `kill`, `journal` and `renice` are the same action against the same process:
/// three spawners that each carried the same seven scalars, and adding the
/// eighth -- the pane size -- put them over the arity clippy enforces. A
/// suppression would have hidden the fact that the three had grown a field
/// together, which is the thing worth seeing. Grouped instead.
///
/// The size is in here for the reason it is in [`ExecAction::dims`]: it is the
/// window the child's output is drawn into, and these three draw into a pane.
pub struct ProcessAction {
    pub idx: usize,
    pub gen: u64,
    pub server: Server,
    pub pid: u32,
    pub name: String,
    pub pass: Option<String>,
    pub dims: (u16, u16),
}

/// `kill -9` the process, on the panel that listed it.
///
/// `success: false` on all three, unchanged from before the group existed: a
/// kill, a journal follow and a renice are actions whose outcome nobody reads a
/// boolean for, and the note carries what did happen.
#[must_use]
pub fn spawn_kill(action: ProcessAction, tx: Sender<Msg>) -> JoinHandle<()> {
    let ProcessAction { idx, gen, .. } = &action;
    let (idx, gen) = (*idx, *gen);
    tokio::spawn(async move {
        let outcome = run_kill(action, &tx).await;
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

/// Follow one unit's journal, in this panel.
#[must_use]
pub fn spawn_journal(action: ProcessAction, tx: Sender<Msg>) -> JoinHandle<()> {
    let ProcessAction { idx, gen, .. } = &action;
    let (idx, gen) = (*idx, *gen);
    tokio::spawn(async move {
        let outcome = run_journal(action, &tx).await;
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

/// Renice the process, in this panel.
#[must_use]
pub fn spawn_renice(action: ProcessAction, tx: Sender<Msg>) -> JoinHandle<()> {
    let ProcessAction { idx, gen, .. } = &action;
    let (idx, gen) = (*idx, *gen);
    tokio::spawn(async move {
        let outcome = run_renice(action, &tx).await;
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
    dims: (u16, u16),
    tx: Sender<Msg>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let outcome = run_tail(idx, gen, &server, pass.as_deref(), dims, &tx).await;
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
///
/// The live size rather than a snapshot, because this one repeats: a terminal
/// resized while the panel is up has to reach the *next* poll, and a size
/// captured at spawn would be the size the panel had when the session started.
/// The Ops poll is the same shape and takes the same receiver.
#[must_use]
pub fn spawn_custom(
    idx: usize,
    gen: u64,
    server: Server,
    command: String,
    pass: Option<String>,
    dims: std::sync::Arc<tokio::sync::watch::Receiver<(u16, u16)>>,
    tx: Sender<Msg>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_millis(
            CUSTOM_PANEL_POLL_INTERVAL_MS,
        ));
        loop {
            interval.tick().await;
            // Read and drop the borrow before awaiting: holding a `watch::Ref`
            // across the await is what makes this future non-`Send`, and the
            // value is a `Copy` pair, so there is no reason to hold it.
            let size = *dims.borrow();
            let _ = run_custom_once(idx, gen, &server, &command, pass.as_deref(), size, &tx).await;
        }
    })
}

async fn run_custom_once(
    idx: usize,
    gen: u64,
    server: &Server,
    command: &str,
    pass: Option<&str>,
    dims: (u16, u16),
    tx: &Sender<Msg>,
) -> String {
    let header = format!("{command} on {}", server.host);
    generic_exec(&ExecAction {
        idx,
        gen,
        server,
        command,
        pass,
        dims,
        tx,
        header: &header,
        action_desc: "custom",
    })
    .await
}

async fn run_kill(action: ProcessAction, tx: &Sender<Msg>) -> String {
    let ProcessAction {
        idx,
        gen,
        server,
        pid,
        name,
        pass,
        dims,
    } = &action;
    let command = format!("kill -9 {pid}");
    let header = format!("Kill {pid}:{name} on {}", server.host);
    let desc = format!("kill {pid}:{name}");
    generic_exec(&ExecAction {
        idx: *idx,
        gen: *gen,
        server,
        command: &command,
        pass: pass.as_deref(),
        dims: *dims,
        tx,
        header: &header,
        action_desc: &desc,
    })
    .await
}

async fn run_journal(action: ProcessAction, tx: &Sender<Msg>) -> String {
    let ProcessAction {
        idx,
        gen,
        server,
        pid,
        name,
        pass,
        dims,
    } = &action;
    let command = format!(
        "journalctl --no-pager -n 200 -f -u {name}.service 2>/dev/null || journalctl --no-pager -n 200 -f --pid={pid} 2>/dev/null || tail -F /proc/{pid}/fd/1 2>/dev/null || tail -n 200 -F /var/log/syslog"
    );
    let header = format!("Journal {pid}:{name} on {}", server.host);
    let desc = format!("journal {pid}:{name}");
    generic_exec(&ExecAction {
        idx: *idx,
        gen: *gen,
        server,
        command: &command,
        pass: pass.as_deref(),
        dims: *dims,
        tx,
        header: &header,
        action_desc: &desc,
    })
    .await
}

async fn run_renice(action: ProcessAction, tx: &Sender<Msg>) -> String {
    let ProcessAction {
        idx,
        gen,
        server,
        pid,
        name,
        pass,
        dims,
    } = &action;
    let command = format!("renice -n 10 -p {pid}");
    let header = format!("Renice {pid}:{name} on {}", server.host);
    let desc = format!("renice {pid}:{name}");
    generic_exec(&ExecAction {
        idx: *idx,
        gen: *gen,
        server,
        command: &command,
        pass: pass.as_deref(),
        dims: *dims,
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
    dims: (u16, u16),
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
        dims,
        tx,
        header: &header,
        action_desc: "tail",
    })
    .await
}
