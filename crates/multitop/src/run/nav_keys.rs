//! The navigation half of key dispatch: focus, selection, scrolling, sort,
//! graph zoom and the yank. Split from `handle_key.rs` for the line cap; the
//! modal and command stages stay there.

use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use tokio::sync::mpsc::Sender;
use tokio::sync::watch;

use crate::app::{App, Msg};

use super::handle_key::process_keys;
use super::tasks::Tasks;
use multitop_agent::SortBy;

/// Focus, selection, scrolling and the single-key actions that need no view switch.
pub(super) fn focus_and_navigation_keys(
    key: KeyEvent,
    app: &mut App,
    dims: (u16, u16),
    dims_rx: &Arc<watch::Receiver<(u16, u16)>>,
    tx: &Sender<Msg>,
    tasks: &mut Tasks,
) -> bool {
    match key.code {
        // Focus first — `Esc` while zoomed should unzoom, not clear the filter
        // underneath or quit. The focused host is the one the user asked to see
        // alone, and the filter they left behind is still there when they return.
        KeyCode::Esc if app.is_focused() => {
            app.toggle_focus();
            app.rerender_all(dims);
            return true;
        }
        // Esc clears an applied filter before it quits. Quitting on the same
        // key that got you here reads as the app dying, and the panels are
        // already hidden, so there is nothing on screen to explain it.
        KeyCode::Esc if !app.filter_query.trim().is_empty() => {
            app.filter_query.clear();
            clamp_selection_to_filter(app);
            app.persist_state();
            return true;
        }
        KeyCode::Char('/') => {
            app.set_filtering(true);
            app.filter_query.clear();
            return true;
        }
        KeyCode::Esc | KeyCode::Char('q' | 'Q') => {
            app.request_quit();
            return true;
        }
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
            app.request_quit();
            return true;
        }
        KeyCode::Char('e' | 'E') => {
            let load = crate::passwords::open(app, app.selected_panel, false);
            app.dispatch_credential_loads(load, tx);
            return true;
        }
        // The number keys count panes on screen, not entries in the config.
        //
        // They used to index the unfiltered list and clamp to its end, so with
        // `/db` showing one pane, `2` selected a host that was not on screen
        // and every view key after it acted on that host instead. Out of range
        // now does nothing, which is the same answer a click on no pane gets:
        // the two ways of choosing a pane agree.
        KeyCode::Char(c @ '1'..='9') => {
            let slot = (c as usize) - ('1' as usize);
            if let Some(&panel) = app.filtered_indices().get(slot) {
                app.selected_panel = panel;
                app.persist_state();
            }
            return true;
        }
        KeyCode::Char('c' | 'C') => {
            set_sort(app, SortBy::Cpu, dims_rx, tx, tasks);
            return true;
        }
        KeyCode::Char('m' | 'M') => {
            set_sort(app, SortBy::Mem, dims_rx, tx, tasks);
            return true;
        }
        KeyCode::Char('t' | 'T') => {
            app.cycle_theme();
            if let Some(ref path) = app.config_path {
                crate::config::save_theme(path, app.current_theme().name);
            }
            app.rerender_all(dims);
            return true;
        }
        KeyCode::Char('+' | '=') => {
            zoom_graphs(app, dims, 1);
            return true;
        }
        KeyCode::Char('-' | '_') => {
            zoom_graphs(app, dims, -1);
            return true;
        }
        KeyCode::Char('y' | 'Y') => {
            yank_selected_host(app);
            return true;
        }
        KeyCode::Char('H') if !app.is_filtering() => {
            let cmds = app.toggle_alerts(dims);
            for cmd in cmds {
                let _ = cmd;
            }
            app.persist_state();
            return true;
        }
        KeyCode::Char('x' | 'X' | 'o' | 'O' | 'r' | 'R') => {
            process_keys(key, app);
            return true;
        }
        KeyCode::Char('l' | 'L') => {
            // `tail -n 200 -F /var/log/syslog` as framed Exec — Painter+RingLines reuse.
            if app.selected_panel < app.panels.len() {
                let panel = app.selected_panel;
                let gen = app.bump(panel);
                let server = app.panels[panel].server.clone();
                let pass = app.panels[panel].sudo_password.clone();
                let handle = crate::tasks::spawn_tail(panel, gen, server, pass, tx.clone());
                tasks.set_aux(panel, handle);
            }
            return true;
        }
        KeyCode::Enter | KeyCode::Char('z' | 'Z') => {
            app.toggle_focus();
            app.rerender_all(dims);
            return true;
        }
        _ => {}
    }

    scroll_keys(key, app)
}

/// The scrolling keys: a line, a page, or the ends of the pane.
fn scroll_keys(key: KeyEvent, app: &mut App) -> bool {
    match key.code {
        KeyCode::Up | KeyCode::Char('k' | 'K') => app.scroll_up(1),
        KeyCode::Down | KeyCode::Char('j' | 'J') => app.scroll_down(1),
        KeyCode::PageUp => app.scroll_up(15),
        KeyCode::PageDown => app.scroll_down(15),
        KeyCode::Home => app.scroll_to_top(),
        KeyCode::End => app.scroll_to_bottom(),
        _ => return false,
    }
    true
}

/// Sort by `sort`; a change persists and restarts every agent with the new
/// order.
fn set_sort(
    app: &mut App,
    sort: SortBy,
    dims_rx: &Arc<watch::Receiver<(u16, u16)>>,
    tx: &Sender<Msg>,
    tasks: &mut Tasks,
) {
    let old_sort = app.sort;
    app.sort = sort;
    if old_sort != app.sort {
        app.persist_state();
        super::event_loop::restart_all_agents(app, dims_rx, tx, tasks);
    }
}

/// Step the graph zoom by `by` in the graph and alert views (1..=16: 4096
/// samples is about 2.2 h; 80 cols * 2 * 16 = 2560 is about 1.4 h, so 1..16
/// covers ~10 s to ~1 h at 80 cols, validated against bench).
fn zoom_graphs(app: &mut App, dims: (u16, u16), by: i8) {
    if app.in_graphs() || app.in_alerts() {
        app.graph_zoom = if by > 0 {
            (app.graph_zoom + 1).clamp(1, 16)
        } else {
            app.graph_zoom.saturating_sub(1).max(1)
        };
        app.rerender_all(dims);
    }
}

pub(super) fn yank_selected_host(app: &App) {
    let Some(panel) = app.panels.get(app.selected_panel) else {
        return;
    };
    let target = panel.server.target();
    let text = target.as_ref().to_string();
    // Try pbcopy (macOS) then xclip/xsel (Linux), best effort.
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("pbcopy")
            .stdin(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut child| {
                use std::io::Write as _;
                if let Some(stdin) = child.stdin.as_mut() {
                    let _ = stdin.write_all(text.as_bytes());
                }
                child.wait()
            });
    }
    #[cfg(target_os = "linux")]
    {
        for prog in ["xclip", "xsel"] {
            let mut cmd = std::process::Command::new(prog);
            if prog == "xclip" {
                cmd.args(["-selection", "clipboard"]);
            }
            if let Ok(mut child) = cmd.stdin(std::process::Stdio::piped()).spawn() {
                use std::io::Write as _;
                if let Some(stdin) = child.stdin.as_mut() {
                    let _ = stdin.write_all(text.as_bytes());
                }
                let _ = child.wait();
                break;
            }
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = text;
    }
}

/// Keep the selection on a panel the user can actually see.
///
/// The selected panel drives the keybar's mode badge and every view-switching
/// key. Left pointing at a filtered-out host, those keys act on a panel that is
/// not on screen.
pub(super) fn clamp_selection_to_filter(app: &mut App) {
    let shown = app.filtered_indices();
    if !shown.is_empty() && !shown.contains(&app.selected_panel) {
        app.selected_panel = shown[0];
    }
}
