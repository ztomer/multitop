//! Application state.
//!
//! Every state transition here is a pure function of the current state plus
//! one message, and returns the side effects it wants performed. The async
//! runtime in `run.rs` does the I/O; this module can be tested without a
//! terminal or a network.

use crate::config::Server;

pub use crate::panel::{Mode, Panel};
pub use crate::types::{Command, Msg};

pub use multitop_agent::SortBy;

pub struct App {
    pub panels: Vec<Panel>,
    pub selected_panel: usize,
    pub should_quit: bool,
    pub sort: SortBy,
    pub theme_idx: usize,
    pub config_path: Option<std::path::PathBuf>,
    pub filter_query: String,
    pub is_filtering: bool,
    pub sparklines: Vec<crate::sparkline::SparklineHistory>,
    pub sparklines_mem: Vec<crate::sparkline::SparklineHistory>,
    pub sparklines_cpu: Vec<crate::sparkline::SparklineHistory>,
    pub upgrade_history_lines: usize,
    pub password_manager: Option<crate::passwords::PasswordManager>,
    pub show_upgrade_modal: bool,
    pub had_upgrade: bool,
    pub upgrades_in_flight: usize,
    pub last_update: Option<String>,
    pub show_sparklines: bool,
}

impl App {
    pub fn new(servers: Vec<Server>) -> Self {
        let count = servers.len();
        App {
            panels: servers.into_iter().map(Panel::new).collect(),
            selected_panel: 0,
            should_quit: false,
            sort: SortBy::Cpu,
            theme_idx: 0,
            config_path: None,
            filter_query: String::new(),
            is_filtering: false,
            sparklines: (0..count)
                .map(|_| crate::sparkline::SparklineHistory::new(30))
                .collect(),
            sparklines_mem: (0..count)
                .map(|_| crate::sparkline::SparklineHistory::new(30))
                .collect(),
            sparklines_cpu: (0..count)
                .map(|_| crate::sparkline::SparklineHistory::new(30))
                .collect(),
            upgrade_history_lines: crate::config::DEFAULT_UPGRADE_HISTORY_LINES,
            password_manager: None,
            show_upgrade_modal: false,
            had_upgrade: false,
            upgrades_in_flight: 0,
            last_update: None,
            show_sparklines: false,
        }
    }

    pub fn filtered_indices(&self) -> Vec<usize> {
        if self.filter_query.trim().is_empty() {
            (0..self.panels.len()).collect()
        } else {
            let q = self.filter_query.to_lowercase();
            self.panels
                .iter()
                .enumerate()
                .filter(|(_, p)| {
                    p.server.host.to_lowercase().contains(&q)
                        || p.server.user.to_lowercase().contains(&q)
                })
                .map(|(i, _)| i)
                .collect()
        }
    }

    pub fn bump(&mut self, idx: usize) -> u64 {
        let p = &mut self.panels[idx];
        p.gen += 1;
        p.gen
    }

    pub fn in_docker(&self) -> bool {
        self.panels.iter().any(|p| p.mode == Mode::Docker)
    }

    pub fn in_fetch(&self) -> bool {
        self.panels.iter().any(|p| p.mode == Mode::Fetch)
    }

    pub fn in_upgrade(&self) -> bool {
        self.panels.iter().any(|p| p.mode == Mode::Upgrade)
    }

    /// `f`: all panels into the Fastfetch view.
    pub fn toggle_fetch(&mut self) -> Vec<Command> {
        self.reset_scroll();
        if self.in_fetch() {
            return Vec::new();
        }
        let pal = self.current_theme();
        let mut cmds = Vec::with_capacity(self.panels.len());
        for i in 0..self.panels.len() {
            let gen = self.bump(i);
            let p = &mut self.panels[i];
            p.mode = Mode::Fetch;
            p.view = vec![format!(
                "{}\u{2192} Fetching system info...{}",
                pal.meter_mid(),
                pal.reset
            )];
            cmds.push(Command::RunFetch { panel: i, gen });
        }
        cmds
    }

    /// `d`: all panels into the Docker view.
    pub fn toggle_docker(&mut self) -> Vec<Command> {
        self.reset_scroll();
        if self.in_docker() {
            return Vec::new();
        }
        let pal = self.current_theme();
        let mut cmds = Vec::with_capacity(self.panels.len());
        for i in 0..self.panels.len() {
            let gen = self.bump(i);
            let p = &mut self.panels[i];
            p.mode = Mode::Docker;
            p.view = vec![format!(
                "{}\u{2192} Docker loading...{}",
                pal.meter_mid(),
                pal.reset
            )];
            cmds.push(Command::RunDocker { panel: i, gen });
        }
        cmds
    }

    /// `s`: back to the live stats view on every panel.
    pub fn switch_stats(&mut self) -> Vec<Command> {
        self.reset_scroll();
        for i in 0..self.panels.len() {
            self.bump(i);
            let p = &mut self.panels[i];
            if p.mode == Mode::Upgrade {
                p.last_upgrade = std::mem::take(&mut p.view);
            }
            p.mode = Mode::Monitor;
            p.show_last_frame();
        }
        Vec::new()
    }

    /// Show the last upgrade output without re-running upgrades.
    pub fn show_upgrade_output(&mut self) {
        self.reset_scroll();
        let pal = self.current_theme();
        for p in &mut self.panels {
            p.mode = Mode::Upgrade;
            p.view = if p.last_upgrade.is_empty() {
                vec![format!(
                    "{}\u{2192} No previous upgrade output{}",
                    pal.meter_mid(),
                    pal.reset
                )]
            } else {
                std::mem::take(&mut p.last_upgrade)
            };
        }
    }

    /// `u`: run each server's configured upgrade command.
    pub fn run_upgrade(&mut self) -> Vec<Command> {
        self.had_upgrade = true;
        self.upgrades_in_flight = self
            .panels
            .iter()
            .filter(|p| p.server.upgrade_cmd.is_some())
            .count();
        self.reset_scroll();
        let pal = self.current_theme();
        let mut cmds = Vec::new();
        for i in 0..self.panels.len() {
            let gen = self.bump(i);
            let p = &mut self.panels[i];
            p.mode = Mode::Upgrade;
            p.ensure_sudo_password();
            match p.server.upgrade_cmd.is_some() {
                true => {
                    p.view = vec![
                        format!(
                            "{}\u{2192} Upgrade running...{}",
                            pal.meter_mid(),
                            pal.reset
                        ),
                        format!(
                            "{}\u{2192} Do not exit (Q) until upgrade completes{}",
                            pal.meter_mid(),
                            pal.reset
                        ),
                    ];
                    cmds.push(Command::RunUpgrade { panel: i, gen });
                }
                false => {
                    p.view = vec![format!(
                        "{}No upgrade_cmd configured for this server{}\n",
                        pal.meter_mid(),
                        pal.reset
                    )];
                }
            }
        }
        cmds
    }

    /// Confirm upgrade from modal and execute `run_upgrade`.
    pub fn confirm_upgrade(&mut self) -> Vec<Command> {
        self.show_upgrade_modal = false;
        let ts = current_timestamp_str();
        self.last_update = Some(ts.clone());
        if let Some(ref path) = self.config_path {
            let state = crate::state::AppState {
                last_update: Some(ts),
            };
            let _ = crate::state::save_state(path, &state);
        }
        self.run_upgrade()
    }

    pub fn quit(&mut self) {
        self.should_quit = true;
    }

    /// True when a message is still relevant to the panel it targets.
    fn accepts(&self, panel: usize, gen: u64) -> bool {
        self.panels.get(panel).is_some_and(|p| p.gen == gen)
    }

    pub fn apply(&mut self, msg: Msg) {
        match msg {
            Msg::Packet {
                panel,
                gen,
                payload,
                dims,
            } => {
                let pal = self.current_theme();
                let sort = self.sort;
                let accepts = self.accepts(panel, gen);
                let Some(p) = self.panels.get_mut(panel) else {
                    return;
                };

                match &payload {
                    multitop_agent::proto::Payload::Monitor(snap) => {
                        p.last_monitor = Some(payload.clone());
                        if panel < self.sparklines_cpu.len() {
                            self.sparklines_cpu[panel].push(snap.cpu_pct as f32);
                            let mem_pct = if snap.mem.total > 0 {
                                (snap.mem.used as f32 / snap.mem.total as f32) * 100.0
                            } else {
                                0.0
                            };
                            self.sparklines_mem[panel].push(mem_pct);
                        }
                        let lines = crate::render_payload::render_payload(&payload, dims, sort, pal);
                        p.last_frame = Some(lines.clone());
                        if p.mode == Mode::Monitor {
                            p.view = lines;
                        }
                    }
                    multitop_agent::proto::Payload::Docker { .. } => {
                        p.last_docker = Some(payload.clone());
                        if p.mode == Mode::Docker && accepts {
                            let lines = crate::render_payload::render_payload(&payload, dims, sort, pal);
                            p.view = lines;
                        }
                    }
                    multitop_agent::proto::Payload::Fetch(snap) => {
                        p.last_fetch = Some(snap.clone());
                        if p.mode == Mode::Fetch && accepts {
                            let lines = crate::fetch_render::render_fetch(
                                snap,
                                dims.0 as usize,
                                dims.1 as usize,
                                pal,
                            );
                            p.view = lines;
                        }
                    }
                }
            }
            Msg::Frame { panel, lines } => {
                let Some(p) = self.panels.get_mut(panel) else {
                    return;
                };
                p.last_frame = Some(lines);
                // Only paint it if stats is what the user is looking at.
                if p.mode == Mode::Monitor {
                    p.show_last_frame();
                }
            }
            Msg::Status { panel, gen, text } => {
                if self.accepts(panel, gen) {
                    self.panels[panel].view = vec![text];
                }
            }
            Msg::FetchData {
                panel,
                gen,
                snap,
                lines,
            } => {
                if self.accepts(panel, gen) {
                    self.panels[panel].last_fetch = Some(snap);
                    self.panels[panel].view = lines;
                }
            }
            Msg::AuxBegin { panel, gen, header } => {
                if self.accepts(panel, gen) {
                    self.panels[panel].view = header.into_iter().collect();
                }
            }
            Msg::AuxLine { panel, gen, line } => {
                if !self.accepts(panel, gen) {
                    return;
                }
                let cap = self.upgrade_history_lines;
                let view = &mut self.panels[panel].view;
                view.push(line);
                if view.len() > cap {
                    view.drain(..view.len() - cap);
                }
            }
            Msg::AuxDone { panel, gen, note } => {
                if self.accepts(panel, gen) {
                    if self.upgrades_in_flight > 0 {
                        self.upgrades_in_flight -= 1;
                    }
                    if let Some(note) = note {
                        self.panels[panel].view.push(note);
                    }
                }
            }
        }
    }

    pub fn scroll_up(&mut self, delta: usize) {
        if self.selected_panel < self.panels.len() {
            let p = &mut self.panels[self.selected_panel];
            let max_scroll = p.view.len().saturating_sub(1);
            p.scroll_offset = (p.scroll_offset + delta).min(max_scroll);
        }
    }

    pub fn scroll_down(&mut self, delta: usize) {
        if self.selected_panel < self.panels.len() {
            let p = &mut self.panels[self.selected_panel];
            p.scroll_offset = p.scroll_offset.saturating_sub(delta);
        }
    }

    pub fn scroll_panel_up(&mut self, panel: usize, delta: usize) {
        if panel < self.panels.len() {
            let p = &mut self.panels[panel];
            let max_scroll = p.view.len().saturating_sub(1);
            p.scroll_offset = (p.scroll_offset + delta).min(max_scroll);
        }
    }

    pub fn scroll_panel_down(&mut self, panel: usize, delta: usize) {
        if panel < self.panels.len() {
            let p = &mut self.panels[panel];
            p.scroll_offset = p.scroll_offset.saturating_sub(delta);
        }
    }

    pub fn scroll_to_top(&mut self) {
        if self.selected_panel < self.panels.len() {
            let p = &mut self.panels[self.selected_panel];
            p.scroll_offset = p.scroll_offset.saturating_sub(1);
        }
    }

    pub fn reset_scroll(&mut self) {
        if self.selected_panel < self.panels.len() {
            self.panels[self.selected_panel].scroll_offset = 0;
        }
    }

    pub fn cycle_theme(&mut self) {
        self.theme_idx = (self.theme_idx + 1) % multitop_agent::color::THEMES.len();
    }

    pub fn current_theme(&self) -> &'static multitop_agent::color::Palette {
        &multitop_agent::color::THEMES[self.theme_idx]
    }

    /// Re-render all panels in their current mode (Stats, Docker, Fetch) at the given dimensions using active theme.
    pub fn rerender_all(&mut self, dims: (u16, u16)) {
        let pal = self.current_theme();
        let sort = self.sort;
        for panel in &mut self.panels {
            match panel.mode {
                Mode::Monitor => {
                    if let Some(payload) = &panel.last_monitor {
                        panel.view = crate::render_payload::render_payload(payload, dims, sort, pal);
                    }
                }
                Mode::Docker => {
                    if let Some(payload) = &panel.last_docker {
                        panel.view = crate::render_payload::render_payload(payload, dims, sort, pal);
                    }
                }
                Mode::Fetch => {
                    if let Some(snap) = &panel.last_fetch {
                        panel.view = crate::fetch_render::render_fetch(
                            snap,
                            dims.0 as usize,
                            dims.1 as usize,
                            pal,
                        );
                    }
                }
                Mode::Upgrade => {}
            }
        }
    }
}

/// Format an error for display inside a panel.
pub fn error_line(text: impl std::fmt::Display) -> String {
    let pal = &multitop_agent::color::ANSI;
    format!("{}{text}{}", pal.meter_high(), pal.reset)
}

pub fn status_line(text: impl std::fmt::Display) -> String {
    let pal = &multitop_agent::color::ANSI;
    format!("{}{text}{}", pal.meter_mid(), pal.reset)
}

pub fn header_line(text: impl std::fmt::Display) -> String {
    let pal = &multitop_agent::color::ANSI;
    format!("{}{}{text}{}", pal.primary(), pal.bold, pal.reset)
}

pub fn current_timestamp_str() -> String {
    use std::time::SystemTime;
    let secs = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let days = secs / 86400;
    let rem_secs = secs % 86400;
    let hours = rem_secs / 3600;
    let minutes = (rem_secs % 3600) / 60;
    let seconds = rem_secs % 60;

    let z = days + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };

    format!("{y:04}-{m:02}-{d:02} {hours:02}:{minutes:02}:{seconds:02} UTC")
}

