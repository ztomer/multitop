//! The Ops view (servers ROADMAP 12.17b): per host, what its `mcp_host`
//! says about health, cron, containers and alerts - the part of a host's
//! state the agent does not stream.

use crate::app::{App, Command};
use crate::ops::OpsState;
use crate::panel::{Mode, Panel, UpgradeState};

/// The frame for `panel` in the Ops view at `dims`. A host with no `mcp`
/// command says how to add one; one asked and not yet answered says so.
#[must_use]
pub fn frame(panel: &Panel, dims: (u16, u16), pal: &multitop_agent::color::Palette) -> Vec<String> {
    let state = match (&panel.server.mcp, &panel.last_ops) {
        (None, _) => &OpsState::NotConfigured,
        (Some(_), Some(s)) => s,
        (Some(_), None) => &OpsState::Asking,
    };
    crate::ops::render::render(
        &panel.server.host,
        state,
        dims.0 as usize,
        dims.1 as usize,
        crate::tasks::ops_poll::now_epoch(),
        pal,
    )
}

impl App {
    #[must_use]
    pub fn in_ops(&self) -> bool {
        self.panels.iter().any(|p| p.mode == Mode::Ops)
    }

    /// Every panel to the Ops view: a poll for each host with an `mcp`
    /// command (showing its last answer, or "asking", until the first
    /// arrives), and for a host without one, the line saying how to add it.
    pub fn toggle_ops(&mut self, dims: (u16, u16)) -> Vec<Command> {
        if self.in_ops() {
            return Vec::new();
        }
        self.leave_current_view();
        let pal = self.current_theme();
        let mut cmds = Vec::new();
        for i in 0..self.panels.len() {
            // A panel mid-upgrade keeps its gen, or its run's output is lost.
            if self.panels[i].upgrade_state != UpgradeState::STARTED {
                self.bump(i);
            }
            let p = &mut self.panels[i];
            p.mode = Mode::Ops;
            if p.server.mcp.is_some() {
                cmds.push(Command::RunOps {
                    panel: i,
                    gen: p.gen,
                });
            }
            let lines = frame(p, dims, pal);
            p.show_frame(lines);
        }
        cmds
    }

    /// A poll's answer: kept, and drawn if the panel is still showing Ops.
    pub(super) fn on_ops(
        &mut self,
        panel: usize,
        gen: u64,
        state: OpsState,
        dims: (u16, u16),
    ) -> bool {
        if !self.accepts(panel, gen) {
            return false;
        }
        let pal = self.current_theme();
        let p = &mut self.panels[panel];
        p.last_ops = Some(state);
        if p.mode == Mode::Ops {
            let lines = frame(p, dims, pal);
            p.show_frame(lines);
        }
        true
    }
}
