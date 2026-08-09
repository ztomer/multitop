//! The process entry point: the real terminal, the real event stream.
//!
//! Kept in a file of its own because none of it can run under test. It takes
//! the terminal over with `ratatui::init`, reads the process's own stdin, and
//! restores both on the way out — a suite that ran it would be fighting the
//! developer for the terminal it is printing into. Everything that *can* be
//! decided without a terminal lives in the sibling modules and is tested
//! there; this is the wiring between them.
//!
//! `tools/coverage_check.sh` excludes this file for that reason.

use std::path::PathBuf;

use crossterm::event::EventStream;
use tokio::sync::watch;

use crate::config::Server;
use crate::ui;

use super::event_loop::event_loop;
use super::terminal::{self, TerminalGuard};

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
