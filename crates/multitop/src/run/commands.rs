//! Carrying out what an `App` method asked for: each command's task, in
//! the slot it belongs in.

use tokio::sync::mpsc::Sender;

use super::tasks::Tasks;
use crate::app::{App, Command, Msg};

/// Carry out the commands an `App` method produced.
pub fn execute_cmds(
    cmds: Vec<Command>,
    app: &App,
    dims: (u16, u16),
    tx: &Sender<Msg>,
    tasks: &mut Tasks,
) {
    for cmd in cmds {
        // A view task supersedes the last view task; an upgrade goes in the
        // upgrade slot and outlives every view switch. Which slot it lands in
        // is decided here, once, rather than by a flag each caller has to keep
        // in step.
        match cmd {
            Command::RunFetch { panel, gen } => tasks.set_aux(
                panel,
                crate::tasks::spawn_fetch(
                    panel,
                    gen,
                    app.panels_epoch,
                    app.panels[panel].server.clone(),
                    dims,
                    app.sort,
                    tx.clone(),
                ),
            ),
            Command::RunDocker { panel, gen } => tasks.set_aux(
                panel,
                crate::tasks::spawn_docker(
                    panel,
                    gen,
                    app.panels_epoch,
                    app.panels[panel].server.clone(),
                    dims,
                    app.sort,
                    tx.clone(),
                ),
            ),
            Command::RunOps { panel, gen } => {
                if let Some(command) = app.panels[panel].server.mcp.clone() {
                    let dims = tasks.dims_or(dims);
                    tasks.set_ops(
                        panel,
                        crate::tasks::spawn_ops(
                            panel,
                            gen,
                            app.panels[panel].server.clone(),
                            command,
                            dims,
                            tx.clone(),
                        ),
                    );
                }
            }
            Command::RunUpgrade { panel, gen } => {
                // Use the panel's stored sudo password (from keychain)
                let password = app.panels[panel].sudo_password.clone();
                tasks.set_upgrade(
                    panel,
                    crate::tasks::spawn_upgrade(
                        panel,
                        gen,
                        app.panels[panel].server.clone(),
                        password,
                        tx.clone(),
                    ),
                );
            }
        }
    }
}
