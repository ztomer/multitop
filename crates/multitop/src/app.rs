//! Application state.
//!
//! Every state transition here is a pure function of the current state plus
//! one message, and returns the side effects it wants performed. The async
//! runtime in `run.rs` does the I/O; this module can be tested without a
//! terminal or a network.

use multitop_agent::fetch::FetchSnapshot;

use crate::config::Server;



/// Upper bound on retained command output, so a chatty `upgrade_cmd` cannot
/// grow the buffer without limit. The view shows the tail regardless.
const MAX_AUX_LINES: usize = 2000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Monitor,
    Docker,
    Fetch,
    Upgrade,
}

#[derive(Clone, Debug)]
pub struct Panel {
    pub server: Server,
    pub mode: Mode,
    pub gen: u64,
    pub last_frame: Option<Vec<String>>,
    pub last_fetch: Option<FetchSnapshot>,
    pub last_monitor: Option<multitop_agent::proto::Payload>,
    pub last_docker: Option<multitop_agent::proto::Payload>,
    pub view: Vec<String>,
}

impl Panel {
    fn new(server: Server) -> Self {
        let pal = &multitop_agent::color::ANSI;
        Panel {
            server,
            mode: Mode::Monitor,
            gen: 0,
            last_frame: None,
            last_fetch: None,
            last_monitor: None,
            last_docker: None,
            view: vec![format!("{}connecting...{}", pal.muted(), pal.reset)],
        }
    }

    fn show_last_frame(&mut self) {
        let pal = &multitop_agent::color::ANSI;
        self.view = match &self.last_frame {
            Some(f) => f.clone(),
            None => vec![format!("{}waiting for data...{}", pal.meter_mid(), pal.reset)],
        };
    }
}

/// Work the runtime should start as a result of a state transition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    RunDocker { panel: usize, gen: u64 },
    RunFetch { panel: usize, gen: u64 },
    RunUpgrade { panel: usize, gen: u64 },
}

/// Messages produced by the background tasks.
#[derive(Clone, Debug, PartialEq)]
pub enum Msg {
    Packet {
        panel: usize,
        gen: u64,
        payload: multitop_agent::proto::Payload,
        dims: (u16, u16),
    },
    /// A monitor frame. Never generation-gated: the monitor stream keeps
    /// running through mode switches so its data stays warm.
    Frame { panel: usize, lines: Vec<String> },
    /// Replace a panel's contents with a transient status line.
    Status {
        panel: usize,
        gen: u64,
        text: String,
    },
    /// Fetch data arrived — store the raw snapshot and its rendered view.
    FetchData {
        panel: usize,
        gen: u64,
        snap: FetchSnapshot,
        lines: Vec<String>,
    },
    /// Begin collecting command output, optionally under a header.
    AuxBegin {
        panel: usize,
        gen: u64,
        header: Option<String>,
    },
    AuxLine {
        panel: usize,
        gen: u64,
        line: String,
    },
    AuxDone {
        panel: usize,
        gen: u64,
        note: Option<String>,
    },
}

pub use multitop_agent::SortBy;

pub struct App {
    pub panels: Vec<Panel>,
    pub should_quit: bool,
    pub sort: SortBy,
    pub theme_idx: usize,
    pub config_path: Option<std::path::PathBuf>,
    pub filter_query: String,
    pub is_filtering: bool,
    pub sparklines: Vec<crate::sparkline::SparklineHistory>,
}

impl App {
    pub fn new(servers: Vec<Server>) -> Self {
        let count = servers.len();
        App {
            panels: servers.into_iter().map(Panel::new).collect(),
            should_quit: false,
            sort: SortBy::Cpu,
            theme_idx: 0,
            config_path: None,
            filter_query: String::new(),
            is_filtering: false,
            sparklines: (0..count).map(|_| crate::sparkline::SparklineHistory::new(30)).collect(),
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

    fn bump(&mut self, idx: usize) -> u64 {
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

    /// `f`: all panels into the Fastfetch view, or all back to stats.
    pub fn toggle_fetch(&mut self) -> Vec<Command> {
        if self.in_fetch() {
            return self.switch_stats();
        }
        let pal = self.current_theme();
        let mut cmds = Vec::with_capacity(self.panels.len());
        for i in 0..self.panels.len() {
            let gen = self.bump(i);
            let p = &mut self.panels[i];
            p.mode = Mode::Fetch;
            p.view = vec![format!("{}\u{2192} Fetching system info...{}", pal.meter_mid(), pal.reset)];
            cmds.push(Command::RunFetch { panel: i, gen });
        }
        cmds
    }

    /// `d`: all panels into the Docker view, or all back to stats.
    ///
    /// The toggle is global rather than per-panel so the screen never shows a
    /// mix of two different views.
    pub fn toggle_docker(&mut self) -> Vec<Command> {
        if self.in_docker() {
            return self.switch_stats();
        }
        let pal = self.current_theme();
        let mut cmds = Vec::with_capacity(self.panels.len());
        for i in 0..self.panels.len() {
            let gen = self.bump(i);
            let p = &mut self.panels[i];
            p.mode = Mode::Docker;
            p.view = vec![format!("{}\u{2192} Docker loading...{}", pal.meter_mid(), pal.reset)];
            cmds.push(Command::RunDocker { panel: i, gen });
        }
        cmds
    }

    /// `s`: back to the live stats view on every panel.
    pub fn switch_stats(&mut self) -> Vec<Command> {
        for i in 0..self.panels.len() {
            self.bump(i);
            let p = &mut self.panels[i];
            p.mode = Mode::Monitor;
            p.show_last_frame();
        }
        Vec::new()
    }

    /// `u`: run each server's configured upgrade command.
    pub fn run_upgrade(&mut self) -> Vec<Command> {
        let pal = self.current_theme();
        let mut cmds = Vec::new();
        for i in 0..self.panels.len() {
            let gen = self.bump(i);
            let p = &mut self.panels[i];
            p.mode = Mode::Upgrade;
            match p.server.upgrade_cmd.is_some() {
                true => {
                    p.view = vec![format!("{}\u{2192} Upgrade running...{}", pal.meter_mid(), pal.reset)];
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

    pub fn quit(&mut self) {
        self.should_quit = true;
    }

    /// True when a message is still relevant to the panel it targets.
    fn accepts(&self, panel: usize, gen: u64) -> bool {
        self.panels.get(panel).is_some_and(|p| p.gen == gen)
    }

    pub fn apply(&mut self, msg: Msg) {
        match msg {
            Msg::Packet { panel, gen, payload, dims } => {
                let pal = self.current_theme();
                let sort = self.sort;
                let accepts = self.accepts(panel, gen);
                let Some(p) = self.panels.get_mut(panel) else { return; };

                match &payload {
                    multitop_agent::proto::Payload::Monitor(_) => {
                        p.last_monitor = Some(payload.clone());
                        let lines = crate::run::render_payload(&payload, dims, sort, pal);
                        p.last_frame = Some(lines.clone());
                        if p.mode == Mode::Monitor {
                            p.view = lines;
                        }
                    }
                    multitop_agent::proto::Payload::Docker { .. } => {
                        p.last_docker = Some(payload.clone());
                        if p.mode == Mode::Docker && accepts {
                            let lines = crate::run::render_payload(&payload, dims, sort, pal);
                            p.view = lines;
                        }
                    }
                    multitop_agent::proto::Payload::Fetch(snap) => {
                        p.last_fetch = Some(snap.clone());
                        if p.mode == Mode::Fetch && accepts {
                            let lines = crate::fetch_render::render_fetch(snap, dims.0 as usize, dims.1 as usize, pal);
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
            Msg::FetchData { panel, gen, snap, lines } => {
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
                let view = &mut self.panels[panel].view;
                view.push(line);
                if view.len() > MAX_AUX_LINES {
                    view.drain(..view.len() - MAX_AUX_LINES);
                }
            }
            Msg::AuxDone { panel, gen, note } => {
                if let (true, Some(note)) = (self.accepts(panel, gen), note) {
                    self.panels[panel].view.push(note);
                }
            }
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
                        panel.view = crate::run::render_payload(payload, dims, sort, pal);
                    }
                }
                Mode::Docker => {
                    if let Some(payload) = &panel.last_docker {
                        panel.view = crate::run::render_payload(payload, dims, sort, pal);
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
