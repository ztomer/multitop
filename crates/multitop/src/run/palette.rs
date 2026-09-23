//! The `:` command palette: typed words to the same actions as the keys.

use tokio::sync::mpsc::Sender;

use crate::app::{App, Command, Msg};

use super::commands::execute_cmds;
use super::nav_keys::{clamp_selection_to_filter, yank_selected_host};
use super::tasks::Tasks;
use multitop_agent::SortBy;

pub(super) fn execute_palette_command(
    input: &str,
    app: &mut App,
    dims: (u16, u16),
    tx: &Sender<Msg>,
    tasks: &mut Tasks,
) {
    let input = input.trim().to_lowercase();
    if let Some(stripped) = input.strip_prefix("filter ") {
        app.filter_query = stripped.to_string();
        clamp_selection_to_filter(app);
        app.persist_state();
    } else if input == "filter" || input == "clear filter" {
        app.filter_query.clear();
        clamp_selection_to_filter(app);
        app.persist_state();
    } else if input.starts_with("upgrade") {
        let loads = app.enter_upgrade_view();
        app.dispatch_credential_loads(loads, tx);
    } else if let Some(cmds) = palette_view(&input, app, dims) {
        // The same path the view keys take: one place decides which slot a
        // view task lands in (the Docker/Fetch copies that lived here were
        // twins of `execute_cmds`, and Ops would have been a third).
        execute_cmds(cmds, app, dims, tx, tasks);
        app.persist_state();
    } else if input.starts_with("sort ") {
        let old = app.sort;
        if input.contains("mem") {
            app.sort = SortBy::Mem;
        } else {
            app.sort = SortBy::Cpu;
        }
        if old != app.sort {
            app.persist_state();
        }
    } else if input == "theme" || input.starts_with("theme ") {
        app.cycle_theme();
        if let Some(ref path) = app.config_path {
            crate::config::save_theme(path, app.current_theme().name);
        }
        app.rerender_all(dims);
    } else if input == "add server" || input == "add" {
        let load = crate::passwords::open(app, app.selected_panel, true);
        app.dispatch_credential_loads(load, tx);
    } else if input == "vault unlock" {
        if let Some((vault, epoch)) = app.begin_vault_unlock() {
            drop(crate::run::spawn::spawn_biometric_unlock(
                vault,
                epoch,
                tx.clone(),
            ));
        } else if app.show_vault_password_prompt() {
            // already prompting
        } else {
            app.set_show_upgrade_modal(true);
        }
    } else if input == "yank" || input.starts_with("yank ") || input == "y" || input == "copy" {
        yank_selected_host(app);
    }
}

/// A palette word that switches the view, and the commands the switch made.
pub(super) fn palette_view(input: &str, app: &mut App, dims: (u16, u16)) -> Option<Vec<Command>> {
    Some(match input {
        "docker" => app.toggle_docker(dims),
        "fetch" => app.toggle_fetch(dims),
        "graphs" | "graph" => app.toggle_graphs(dims),
        "stats" | "s" => app.switch_stats(),
        "ops" => app.toggle_ops(dims),
        _ => return None,
    })
}
