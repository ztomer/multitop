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

#[cfg(test)]
mod tests {
    use super::*;

    fn servers(n: usize) -> Vec<Server> {
        (0..n)
            .map(|i| Server {
                host: format!("s{i}"),
                port: 22,
                user: String::new(),
                upgrade_cmd: None,
            })
            .collect()
    }

    fn app(n: usize) -> App {
        App::new(servers(n))
    }

    fn text(p: &Panel) -> String {
        p.view.join("\n")
    }

    #[test]
    fn starts_in_monitor_mode_showing_connecting() {
        let a = app(2);
        assert_eq!(a.panels.len(), 2);
        for p in &a.panels {
            assert_eq!(p.mode, Mode::Monitor);
            assert!(text(p).contains("connecting..."));
        }
    }

    #[test]
    fn empty_server_list_is_allowed() {
        let mut a = app(0);
        assert!(a.panels.is_empty());
        assert!(a.toggle_docker().is_empty());
        assert!(a.switch_stats().is_empty());
    }

    #[test]
    fn frame_is_shown_in_monitor_mode() {
        let mut a = app(1);
        a.apply(Msg::Frame {
            panel: 0,
            lines: vec!["line1".into(), "line2".into()],
        });
        assert_eq!(text(&a.panels[0]), "line1\nline2");
    }

    #[test]
    fn frame_is_stored_but_hidden_in_docker_mode() {
        let mut a = app(1);
        a.toggle_docker();
        a.apply(Msg::Frame {
            panel: 0,
            lines: vec!["fresh".into()],
        });
        assert!(
            !text(&a.panels[0]).contains("fresh"),
            "docker view must not be overwritten"
        );
        assert_eq!(
            a.panels[0].last_frame.as_deref(),
            Some(&["fresh".to_string()][..])
        );
    }

    #[test]
    fn frame_for_unknown_panel_is_ignored() {
        let mut a = app(1);
        a.apply(Msg::Frame {
            panel: 9,
            lines: vec!["x".into()],
        });
        assert!(text(&a.panels[0]).contains("connecting"));
    }

    #[test]
    fn toggle_docker_switches_every_panel_at_once() {
        let mut a = app(3);
        let cmds = a.toggle_docker();
        assert_eq!(cmds.len(), 3);
        for p in &a.panels {
            assert_eq!(p.mode, Mode::Docker);
            assert!(text(p).contains("Docker loading"));
        }
    }

    #[test]
    fn toggle_docker_returns_every_panel_to_monitor() {
        let mut a = app(3);
        a.toggle_docker();
        let cmds = a.toggle_docker();
        assert!(cmds.is_empty());
        for p in &a.panels {
            assert_eq!(p.mode, Mode::Monitor);
        }
    }

    /// The regression the Python version shipped a fix for: after toggling
    /// back, panels that already had data must show it, not "connecting...".
    #[test]
    fn toggling_back_restores_the_last_frame() {
        let mut a = app(3);
        for i in 0..3 {
            a.apply(Msg::Frame {
                panel: i,
                lines: vec![format!("data{i}")],
            });
        }
        a.toggle_docker();
        a.toggle_docker();
        for (i, p) in a.panels.iter().enumerate() {
            assert_eq!(text(p), format!("data{i}"), "panel {i}");
        }
    }

    #[test]
    fn toggling_back_without_data_says_waiting() {
        let mut a = app(1);
        a.toggle_docker();
        a.toggle_docker();
        assert!(text(&a.panels[0]).contains("waiting for data"));
    }

    #[test]
    fn switch_stats_from_docker() {
        let mut a = app(3);
        a.toggle_docker();
        a.switch_stats();
        for p in &a.panels {
            assert_eq!(p.mode, Mode::Monitor);
        }
    }

    #[test]
    fn every_transition_bumps_the_generation() {
        let mut a = app(1);
        let g0 = a.panels[0].gen;
        a.toggle_docker();
        let g1 = a.panels[0].gen;
        a.switch_stats();
        let g2 = a.panels[0].gen;
        assert!(g1 > g0 && g2 > g1);
    }

    #[test]
    fn stale_results_are_dropped() {
        let mut a = app(1);
        let cmds = a.toggle_docker();
        let Command::RunDocker { gen, .. } = cmds[0] else {
            panic!()
        };
        a.switch_stats(); // supersedes the docker request

        a.apply(Msg::AuxBegin {
            panel: 0,
            gen,
            header: None,
        });
        a.apply(Msg::AuxLine {
            panel: 0,
            gen,
            line: "late docker output".into(),
        });
        assert!(!text(&a.panels[0]).contains("late docker output"));
    }

    #[test]
    fn current_results_are_shown() {
        let mut a = app(1);
        let cmds = a.toggle_docker();
        let Command::RunDocker { gen, .. } = cmds[0] else {
            panic!()
        };
        a.apply(Msg::AuxBegin {
            panel: 0,
            gen,
            header: None,
        });
        a.apply(Msg::AuxLine {
            panel: 0,
            gen,
            line: "container list".into(),
        });
        assert_eq!(text(&a.panels[0]), "container list");
    }

    #[test]
    fn aux_output_streams_line_by_line() {
        let mut a = app(1);
        let gen = a.panels[0].gen;
        a.apply(Msg::AuxBegin {
            panel: 0,
            gen,
            header: Some("Upgrade on s0".into()),
        });
        for i in 0..3 {
            a.apply(Msg::AuxLine {
                panel: 0,
                gen,
                line: format!("step {i}"),
            });
        }
        let t = text(&a.panels[0]);
        assert!(t.starts_with("Upgrade on s0"));
        assert!(t.contains("step 0") && t.contains("step 2"));
    }

    #[test]
    fn aux_output_is_capped() {
        let mut a = app(1);
        let gen = a.panels[0].gen;
        a.apply(Msg::AuxBegin {
            panel: 0,
            gen,
            header: None,
        });
        for i in 0..MAX_AUX_LINES + 500 {
            a.apply(Msg::AuxLine {
                panel: 0,
                gen,
                line: format!("l{i}"),
            });
        }
        let view = &a.panels[0].view;
        assert_eq!(view.len(), MAX_AUX_LINES);
        // The tail is what survives — that is where a command's result is.
        assert_eq!(view.last().unwrap(), &format!("l{}", MAX_AUX_LINES + 499));
    }

    #[test]
    fn upgrade_without_command_explains_itself() {
        let mut a = app(1);
        let cmds = a.run_upgrade();
        assert!(cmds.is_empty(), "nothing to run");
        assert!(text(&a.panels[0]).contains("No upgrade_cmd"));
    }

    #[test]
    fn upgrade_with_command_is_scheduled() {
        let mut servers = servers(2);
        servers[0].upgrade_cmd = Some("apt upgrade -y".into());
        let mut a = App::new(servers);
        let cmds = a.run_upgrade();
        assert_eq!(cmds.len(), 1);
        assert!(matches!(cmds[0], Command::RunUpgrade { panel: 0, .. }));
        assert!(text(&a.panels[0]).contains("Upgrade running"));
        assert!(text(&a.panels[1]).contains("No upgrade_cmd"));
    }

    /// Upgrade output must survive on screen. The Python version restored the
    /// monitor frame the instant the command finished, so the result flashed
    /// past unread; here it stays until the user presses `s`.
    #[test]
    fn upgrade_output_persists_until_dismissed() {
        let mut servers = servers(1);
        servers[0].upgrade_cmd = Some("apt upgrade -y".into());
        let mut a = App::new(servers);
        let cmds = a.run_upgrade();
        let Command::RunUpgrade { gen, .. } = cmds[0] else {
            panic!()
        };

        a.apply(Msg::AuxBegin {
            panel: 0,
            gen,
            header: Some("Upgrade on s0".into()),
        });
        a.apply(Msg::AuxLine {
            panel: 0,
            gen,
            line: "42 packages upgraded".into(),
        });
        a.apply(Msg::AuxDone {
            panel: 0,
            gen,
            note: None,
        });
        // A monitor frame arriving afterwards must not clobber it.
        a.apply(Msg::Frame {
            panel: 0,
            lines: vec!["cpu stats".into()],
        });

        assert!(text(&a.panels[0]).contains("42 packages upgraded"));
        a.switch_stats();
        assert_eq!(text(&a.panels[0]), "cpu stats");
    }

    #[test]
    fn status_respects_generation() {
        let mut a = app(1);
        let gen = a.panels[0].gen;
        a.apply(Msg::Status {
            panel: 0,
            gen,
            text: "installing agent".into(),
        });
        assert_eq!(text(&a.panels[0]), "installing agent");
        a.switch_stats();
        a.apply(Msg::Status {
            panel: 0,
            gen,
            text: "stale".into(),
        });
        assert!(!text(&a.panels[0]).contains("stale"));
    }

    #[test]
    fn aux_done_can_append_a_note() {
        let mut a = app(1);
        let gen = a.panels[0].gen;
        a.apply(Msg::AuxBegin {
            panel: 0,
            gen,
            header: None,
        });
        a.apply(Msg::AuxDone {
            panel: 0,
            gen,
            note: Some("exit 1".into()),
        });
        assert!(text(&a.panels[0]).contains("exit 1"));
    }

    #[test]
    fn quit_sets_the_flag() {
        let mut a = app(1);
        assert!(!a.should_quit);
        a.quit();
        assert!(a.should_quit);
    }

    #[test]
    fn helpers_wrap_in_ansi() {
        assert!(error_line("boom").contains("boom"));
        assert!(error_line("boom").starts_with(RED));
        assert!(status_line("wait").starts_with(YELLOW));
        assert!(header_line("hi").contains("hi"));
    }
}
