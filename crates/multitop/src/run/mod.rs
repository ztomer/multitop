//! The async runtime: terminal event loop plus one SSH task per panel.
//!
//! Split into submodules by concern:
//! - [`terminal`] — signal handling, terminal modes, guard
//! - [`tasks`] — per-panel task handles (monitors, aux, upgrades)
//! - [`dims`] — agent render size computation
//! - [`event_loop`] — the core `select!` loop + `panel_at_pos`
//! - [`handle_key`] — key dispatch + `execute_cmds`
//! - [`spawn`] — monitor spawn (re-exported)

pub(super) mod dims;
pub(super) mod event_loop;
pub(super) mod handle_key;
pub(super) mod tasks;
pub(super) mod terminal;

pub mod spawn;

pub use event_loop::{event_loop, panel_at_pos, LoopOutcome};
pub use handle_key::handle_key;
pub use tasks::Tasks;
pub use terminal::SignalAction;
pub use terminal::TerminalGuard;

use std::path::PathBuf;

use crossterm::event::EventStream;
use tokio::sync::watch;

use crate::config::Server;
use crate::ui;

const RESIZE_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(30);
pub(super) const RECONNECT_BACKOFF: [u64; 4] = [2, 5, 10, 20];

/// What one monitor session achieved, from the reconnect loop's point of view.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum SessionOutcome {
    /// The connection was never established.
    NeverConnected,
    /// It connected, and ended without ever delivering a frame.
    NoData,
    /// It delivered at least one frame before ending.
    Delivered,
}

/// How long to wait before reconnecting, and how the failure count moves.
///
/// The count used to be reset the moment `connect` returned -- which is before
/// anything has been received. A host that *accepts* the connection and then
/// fails therefore reset the backoff on every round and was retried at the
/// shortest interval forever: a login banner where the protocol should be, or
/// an agent whose version mismatch cannot be resolved because the upload keeps
/// failing, meant one `ssh` process every two seconds indefinitely -- and in the
/// second case a multi-megabyte agent upload with it.
///
/// Only delivered data says the connection is worth trusting again.
pub(super) fn reconnect_wait(outcome: SessionOutcome, failures: &mut usize) -> u64 {
    if outcome == SessionOutcome::Delivered {
        *failures = 0;
    }
    let wait = RECONNECT_BACKOFF[(*failures).min(RECONNECT_BACKOFF.len() - 1)];
    *failures = failures.saturating_add(1);
    wait
}

/// Run the multitop application.
///
/// # Errors
///
/// Returns an error if the terminal cannot be initialized or the event loop fails.
pub async fn run(
    servers: Vec<Server>,
    config_path: PathBuf,
    initial_theme: Option<String>,
) -> std::io::Result<()> {
    let mut terminal = ratatui::init();
    // Restoration is a guard rather than trailing statements, so it cannot be
    // skipped. Both of the calls that used to sit here propagated with `?`: a
    // failure to *enable* mouse capture returned before the restore, and -- worse
    // -- so did a failure to *disable* it, meaning the cleanup path could skip
    // its own cleanup. Either left the shell in raw mode inside the alternate
    // screen, needing `reset` to recover.
    //
    // Panics are already covered: `ratatui::init` installs a panic hook that
    // restores the terminal, which matters because the release profile aborts
    // and would not run this `Drop`.
    let restore = TerminalGuard;
    // After `ratatui::init`, so ours runs first and ratatui's restore runs
    // second -- the modes come off in the reverse of the order they went on.
    terminal::hook_terminal_modes_into_panics();
    terminal::enter_terminal_modes(&mut std::io::stdout())?;
    let (dims_tx, _) = watch::channel(ui::agent_dims(terminal.size()?, servers.len()));
    let mut events = EventStream::new();
    let outcome = event_loop(
        &mut terminal,
        &mut events,
        dims_tx,
        servers,
        config_path,
        initial_theme,
    )
    .await;
    // Restore the terminal before saying anything on stderr: writing while raw
    // mode and the alternate screen are still up renders the notice into the
    // wreck the shell is about to redraw over.
    drop(restore);
    // Before the error, and unconditionally. This notice used to sit behind a
    // `?` on the loop's result, so the one exit that kills upgrades *without
    // the user asking* -- the terminal going away mid-frame -- was the one exit
    // that said nothing about the transactions it had just interrupted.
    if !outcome.killed.is_empty() {
        eprintln!(
            "multitop quit with upgrades still running on: {}",
            outcome.killed.join(", ")
        );
        eprintln!(
            "  Those SSH sessions were terminated mid-run. The remote lock file\n\
             \x20 on each host may need removing before the next upgrade can start."
        );
    }
    outcome.error.map_or(Ok(()), Err)
}
