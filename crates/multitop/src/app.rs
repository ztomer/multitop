//! Application state.
//!
//! Every state transition here is a pure function of the current state plus
//! one message, and returns the side effects it wants performed. The async
//! runtime in `run.rs` does the I/O; this module can be tested without a
//! terminal or a network.

use crate::config::Server;

/// Status lines carry their own ANSI so a panel's contents are always just
/// "lines of agent-flavoured text", whatever produced them.
const YELLOW: &str = "\x1b[0;33m";
const RED: &str = "\x1b[0;31m";
const GRAY: &str = "\x1b[0;90m";
const RESET: &str = "\x1b[0m";

/// Upper bound on retained command output, so a chatty `upgrade_cmd` cannot
/// grow the buffer without limit. The view shows the tail regardless.
const MAX_AUX_LINES: usize = 2000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Monitor,
    Docker,
    Upgrade,
}

#[derive(Clone, Debug)]
pub struct Panel {
    pub server: Server,
    pub mode: Mode,
    /// Bumped on every mode switch; late results from a superseded request
    /// carry an old generation and are dropped.
    pub gen: u64,
    /// Most recent monitor frame, kept across mode switches so returning to
    /// stats is instant instead of showing "connecting..." again.
    pub last_frame: Option<Vec<String>>,
    pub view: Vec<String>,
}

impl Panel {
    fn new(server: Server) -> Self {
        Panel {
            server,
            mode: Mode::Monitor,
            gen: 0,
            last_frame: None,
            view: vec![format!("{GRAY}connecting...{RESET}")],
        }
    }

    fn show_last_frame(&mut self) {
        self.view = match &self.last_frame {
            Some(f) => f.clone(),
            None => vec![format!("{YELLOW}waiting for data...{RESET}")],
        };
    }
}

/// Work the runtime should start as a result of a state transition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    RunDocker { panel: usize, gen: u64 },
    RunUpgrade { panel: usize, gen: u64 },
}

/// Messages produced by the background tasks.
#[derive(Clone, Debug, PartialEq)]
pub enum Msg {
    /// A monitor frame. Never generation-gated: the monitor stream keeps
    /// running through mode switches so its data stays warm.
    Frame { panel: usize, lines: Vec<String> },
    /// Replace a panel's contents with a transient status line.
    Status {
        panel: usize,
        gen: u64,
        text: String,
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

pub struct App {
    pub panels: Vec<Panel>,
    pub should_quit: bool,
}

impl App {
    pub fn new(servers: Vec<Server>) -> Self {
        App {
            panels: servers.into_iter().map(Panel::new).collect(),
            should_quit: false,
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

    /// `d`: all panels into the Docker view, or all back to stats.
    ///
    /// The toggle is global rather than per-panel so the screen never shows a
    /// mix of two different views.
    pub fn toggle_docker(&mut self) -> Vec<Command> {
        if self.in_docker() {
            return self.switch_stats();
        }
        let mut cmds = Vec::with_capacity(self.panels.len());
        for i in 0..self.panels.len() {
            let gen = self.bump(i);
            let p = &mut self.panels[i];
            p.mode = Mode::Docker;
            p.view = vec![format!("{YELLOW}\u{2192} Docker loading...{RESET}")];
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
        let mut cmds = Vec::new();
        for i in 0..self.panels.len() {
            let gen = self.bump(i);
            let p = &mut self.panels[i];
            p.mode = Mode::Upgrade;
            match p.server.upgrade_cmd.is_some() {
                true => {
                    p.view = vec![format!("{YELLOW}\u{2192} Upgrade running...{RESET}")];
                    cmds.push(Command::RunUpgrade { panel: i, gen });
                }
                false => {
                    p.view = vec![format!(
                        "{YELLOW}No upgrade_cmd configured for this server{RESET}\n"
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
}

/// Format an error for display inside a panel.
pub fn error_line(text: impl std::fmt::Display) -> String {
    format!("{RED}{text}{RESET}")
}

pub fn status_line(text: impl std::fmt::Display) -> String {
    format!("{YELLOW}{text}{RESET}")
}

pub fn header_line(text: impl std::fmt::Display) -> String {
    format!("\x1b[1m{text}{RESET}")
}
