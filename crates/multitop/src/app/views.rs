use crate::app::App;
use crate::app::{AppMode, Confirm, VaultState};
use crate::config::Server;
use crate::panel::{Mode, Panel};
use crate::types::Command;
use multitop_agent::SortBy;
impl App {
    // / The top of the scrollback, stored unclamped.
    pub const SCROLL_TOP: usize = usize::MAX;

    pub fn new(servers: Vec<Server>) -> Self {
        Self {
            panels: servers.into_iter().map(Panel::new).collect(),
            selected_panel: 0,
            mode: AppMode::Running,
            sort: SortBy::Cpu,
            theme_idx: 0,
            config_path: None,
            filter_query: String::new(),
            upgrade_history_lines: crate::config::DEFAULT_UPGRADE_HISTORY_LINES,
            banner_style: crate::layout::BannerStyle::default(),
            password_manager: None,
            last_update: None,
            upgrade_started_at: None,
            vault: None,
            vault_state: VaultState::default(),
            vault_password_input: String::new(),
            vault_epoch: 0,
            host_updates: std::collections::BTreeMap::new(),
            should_quit: false,
            quit_armed: false,
            panels_epoch: 0,
            help_visible: false,
            focused_panel: None,
            command_palette_visible: false,
            command_input: String::new(),
            graph_zoom: 1,
            alert_cpu: None,
            alert_mem: None,
            alert_disk: None,
        }
    }

    pub fn replace_panels(&mut self, servers: Vec<Server>) {
        self.panels_epoch += 1;
        // Generations continue upward instead of restarting at zero. `Frame` is
        // guarded by the epoch, but docker, fetch and upgrade tasks are gated on
        // `gen` -- and a task that outlives the swap still holds the value it was
        // spawned with. Counting a fresh panel back up from zero walks straight
        // through those values, so the first mode switch on the replacement
        // reached generation 1 and made a task spawned for the *old* host
        // acceptable again, on a panel that is now a different machine.
        let next_gen = self.panels.iter().map(|p| p.gen).max().unwrap_or(0) + 1;
        let mut panels: Vec<Panel> = servers.into_iter().map(Panel::new).collect();
        for panel in &mut panels {
            panel.gen = next_gen;
            // `Panel::new` builds the ring at the compiled-in default. The
            // configured `upgrade_history_lines` is applied once at startup, so
            // without this an edit to the server list silently reset every
            // panel's scrollback to the default whatever the config said.
            panel.last_upgrade.set_cap(self.upgrade_history_lines);
            // Carry the credential across when the same account survives the
            // edit, matched on the full identity rather than the host: two
            // entries on one machine with different users or ports are
            // different credentials, and handing the first one's password to
            // the rest would send one account's sudo password to another's
            // session.
            let key = crate::password_store::account(&panel.server);
            if let Some(old) = self
                .panels
                .iter()
                .find(|p| crate::password_store::account(&p.server) == key)
            {
                panel.sudo_password.clone_from(&old.sudo_password);
                panel.password_saved = old.password_saved;
                panel.external_password = old.external_password;
                panel.password_checked = old.password_checked;
            }
        }
        let count = panels.len();
        self.panels = panels;
        self.selected_panel = self.selected_panel.min(count.saturating_sub(1));
    }

    #[must_use]
    pub fn upgrades_in_flight(&self) -> bool {
        self.panels
            .iter()
            .any(|p| p.upgrade_state == crate::panel::UpgradeState::STARTED)
    }

    #[must_use]
    pub fn upgrade_skip_hosts(&self) -> Vec<String> {
        self.filtered_indices()
            .iter()
            .filter(|&&i| self.panels[i].server.upgrade_cmd.is_none())
            .map(|&i| self.panels[i].server.host.clone())
            .collect()
    }

    #[must_use]
    pub const fn should_quit(&self) -> bool {
        self.should_quit
    }

    #[must_use]
    pub const fn is_filtering(&self) -> bool {
        matches!(self.mode, AppMode::Filtering)
    }

    pub fn set_filtering(&mut self, filtering: bool) {
        if filtering {
            self.mode = AppMode::Filtering;
        } else if matches!(self.mode, AppMode::Filtering) {
            self.mode = AppMode::Running;
        }
    }

    #[must_use]
    /// The panels the grid draws, in order.
    ///
    /// What counts as a match is the panel's own question -- it depends on what
    /// that panel is currently showing -- so it is asked rather than answered
    /// here against two fields chosen once.
    pub fn filtered_indices(&self) -> Vec<usize> {
        if let Some(focused) = self.focused_panel {
            if focused < self.panels.len() {
                return vec![focused];
            }
        }
        self.panels
            .iter()
            .enumerate()
            .filter(|(_, p)| p.matches_filter(&self.filter_query))
            .map(|(i, _)| i)
            .collect()
    }

    /// How many panes the grid is currently laying out.
    ///
    /// The number the grid is split by and the number the agent render size is
    /// derived from have to be the same number. They were not: `ui::draw`
    /// splits by `filtered_indices()`, while `agent_dims` was handed
    /// `panels.len()`. Filter four hosts down to one and the pane became the
    /// whole screen while the frame drawn into it stayed a quarter of it.
    #[must_use]
    pub fn visible_panes(&self) -> usize {
        self.filtered_indices().len()
    }

    pub fn toggle_focus(&mut self) {
        if let Some(focused) = self.focused_panel {
            // Unfocus — restore selection to the focused host.
            self.selected_panel = focused;
            self.focused_panel = None;
        } else {
            // Focus the selected panel, even if it is filtered out — focus
            // is the one case where a hidden host is deliberately shown alone.
            self.focused_panel = Some(self.selected_panel);
        }
    }

    #[must_use]
    pub const fn is_focused(&self) -> bool {
        self.focused_panel.is_some()
    }

    pub fn bump(&mut self, idx: usize) -> u64 {
        let p = &mut self.panels[idx];
        p.gen += 1;
        p.gen
    }

    #[must_use]
    pub fn in_docker(&self) -> bool {
        self.panels.iter().any(|p| p.mode == Mode::Docker)
    }

    #[must_use]
    pub fn in_fetch(&self) -> bool {
        self.panels.iter().any(|p| p.mode == Mode::Fetch)
    }

    #[must_use]
    pub fn in_graphs(&self) -> bool {
        self.panels.iter().any(|p| p.mode == Mode::Graphs)
    }

    #[must_use]
    pub fn in_alerts(&self) -> bool {
        self.panels.iter().any(|p| p.mode == Mode::Alerts)
    }

    pub fn toggle_alerts(&mut self, dims: (u16, u16)) -> Vec<Command> {
        if self.in_alerts() {
            return Vec::new();
        }
        self.leave_current_view();
        let pal = self.current_theme();
        for i in 0..self.panels.len() {
            if self.panels[i].upgrade_state != crate::panel::UpgradeState::STARTED {
                self.bump(i);
            }
            let p = &mut self.panels[i];
            p.mode = Mode::Alerts;
            let lines = crate::graphs::render_graphs_with_zoom(
                &p.history,
                dims.0 as usize,
                dims.1 as usize,
                pal,
                self.graph_zoom,
            );
            p.show_frame(lines);
        }
        Vec::new()
    }

    /// Draw every panel's history. The Monitor stream is already running and
    /// already feeding `history`, so there is nothing to spawn and nothing to
    /// wait for -- this is a change of how the same packets are drawn.
    pub fn toggle_graphs(&mut self, dims: (u16, u16)) -> Vec<Command> {
        if self.in_graphs() {
            return Vec::new();
        }
        self.leave_current_view();
        let pal = self.current_theme();
        for i in 0..self.panels.len() {
            // Same rule as `switch_stats`: a panel mid-upgrade keeps its gen,
            // or the in-flight task's output is discarded.
            if self.panels[i].upgrade_state != crate::panel::UpgradeState::STARTED {
                self.bump(i);
            }
            let p = &mut self.panels[i];
            p.mode = Mode::Graphs;
            let lines = crate::graphs::render_graphs_with_zoom(
                &p.history,
                dims.0 as usize,
                dims.1 as usize,
                pal,
                self.graph_zoom,
            );
            p.show_frame(lines);
        }
        Vec::new()
    }

    #[must_use]
    pub fn in_upgrade(&self) -> bool {
        self.panels.iter().any(|p| p.mode == Mode::Upgrade)
    }

    pub fn toggle_fetch(&mut self, dims: (u16, u16)) -> Vec<Command> {
        if self.in_fetch() {
            return Vec::new();
        }
        self.leave_current_view();
        let pal = self.current_theme();
        let mut cmds = Vec::with_capacity(self.panels.len());
        for i in 0..self.panels.len() {
            let gen = self.bump(i);
            let p = &mut self.panels[i];
            p.mode = Mode::Fetch;
            // Show cached data immediately if we have it; the spawned task refreshes.
            if let Some(snap) = &p.last_fetch {
                let lines =
                    crate::fetch_render::render_fetch(snap, dims.0 as usize, dims.1 as usize, pal);
                p.show_frame(lines);
            } else {
                p.show_body(std::iter::once(format!(
                    "{}\u{2192} Fetching system info...{}",
                    pal.meter_mid(),
                    pal.reset
                )));
            }
            cmds.push(Command::RunFetch { panel: i, gen });
        }
        cmds
    }

    pub fn toggle_docker(&mut self, dims: (u16, u16)) -> Vec<Command> {
        if self.in_docker() {
            return Vec::new();
        }
        self.leave_current_view();
        let pal = self.current_theme();
        let mut cmds = Vec::with_capacity(self.panels.len());
        for i in 0..self.panels.len() {
            let gen = self.bump(i);
            let p = &mut self.panels[i];
            p.mode = Mode::Docker;
            // Show cached data immediately if we have it; the spawned task refreshes.
            if let Some(payload) = &p.last_docker {
                let lines = crate::render_payload::render_payload(payload, dims, self.sort, pal);
                p.show_frame(lines);
            } else {
                p.show_body(std::iter::once(format!(
                    "{}\u{2192} Docker loading...{}",
                    pal.meter_mid(),
                    pal.reset
                )));
            }
            cmds.push(Command::RunDocker { panel: i, gen });
        }
        cmds
    }

    pub fn switch_stats(&mut self) -> Vec<Command> {
        self.leave_current_view();
        for i in 0..self.panels.len() {
            // A panel mid-upgrade keeps its gen valid: the in-flight task
            // holds that gen and its output must keep landing in `last_upgrade`.
            // Bumping would retire the gen and discard every line the task
            // sends while the user is looking elsewhere.
            // Only the *generation* has to survive: scroll is handled above,
            // for every panel alike, because an offset into the upgrade log is
            // not an offset into the Monitor pane the user is about to see.
            let started = self.panels[i].upgrade_state == crate::panel::UpgradeState::STARTED;
            if !started {
                self.bump(i);
            }
            let p = &mut self.panels[i];
            // `last_upgrade` is maintained directly by the AuxLine/AuxDone
            // handlers, so it needs no snapshot here. Copying the view into it
            // used to be the only thing preserving the completion marker, and
            // it would now also copy the pane's status header into the log and
            // duplicate it on the way back in.
            p.mode = Mode::Monitor;
            p.show_last_frame();
        }
        Vec::new()
    }

    pub const fn quit(&mut self) {
        self.should_quit = true;
    }

    #[must_use]
    pub const fn quit_armed(&self) -> bool {
        self.quit_armed
    }

    #[must_use]
    pub const fn active_confirm(&self) -> Option<Confirm> {
        if self.quit_armed {
            Some(Confirm::Quit)
        } else if matches!(self.mode, AppMode::ShowUpgradeModal) {
            Some(Confirm::Upgrade)
        } else {
            None
        }
    }

    pub const fn cancel_quit(&mut self) {
        self.quit_armed = false;
    }

    pub fn request_quit(&mut self) {
        if self.quit_armed {
            self.quit_armed = false;
            self.should_quit = true;
        } else if self.upgrades_in_flight() {
            self.quit_armed = true;
        } else {
            self.should_quit = true;
        }
    }

    #[must_use]
    pub fn previous_upgrade_interrupted(&self) -> bool {
        self.upgrade_started_at
            .is_some_and(|started| self.last_update.is_none_or(|last| started > last))
    }

    #[must_use]
    pub fn running_upgrade_hosts(&self) -> Vec<String> {
        self.panels
            .iter()
            .filter(|p| p.upgrade_state == crate::panel::UpgradeState::STARTED)
            .map(|p| p.server.host.clone())
            .collect()
    }

    pub fn scroll_up(&mut self, delta: usize) {
        if let Some(p) = self.panels.get_mut(self.selected_panel) {
            p.scroll_offset = p.scroll_offset.saturating_add(delta);
        }
    }

    pub fn scroll_down(&mut self, delta: usize) {
        if self.selected_panel < self.panels.len() {
            let p = &mut self.panels[self.selected_panel];
            p.scroll_offset = p.scroll_offset.saturating_sub(delta);
        }
    }

    pub fn scroll_panel_up(&mut self, panel: usize, delta: usize) {
        if let Some(p) = self.panels.get_mut(panel) {
            p.scroll_offset = p.scroll_offset.saturating_add(delta);
        }
    }

    pub fn scroll_panel_down(&mut self, panel: usize, delta: usize) {
        if panel < self.panels.len() {
            let p = &mut self.panels[panel];
            p.scroll_offset = p.scroll_offset.saturating_sub(delta);
        }
    }

    pub fn scroll_to_top(&mut self) {
        if let Some(p) = self.panels.get_mut(self.selected_panel) {
            p.scroll_offset = Self::SCROLL_TOP;
        }
    }

    pub fn scroll_to_bottom(&mut self) {
        if let Some(p) = self.panels.get_mut(self.selected_panel) {
            p.scroll_offset = 0;
        }
    }

    /// Leave whatever view the panes are in: park the Upgrade log's offset
    /// where re-entry can find it, and start the next view at its own top.
    ///
    /// Called *after* the "already in this view" guard, never before. Running
    /// it first meant pressing a view's own key — a documented no-op that
    /// spawns nothing — silently threw the user's scroll position away.
    fn leave_current_view(&mut self) {
        for p in &mut self.panels {
            if p.mode == Mode::Upgrade {
                p.upgrade_scroll_offset = p.scroll_offset;
            }
            p.scroll_offset = 0;
        }
    }

    pub fn reset_scroll(&mut self) {
        for panel in &mut self.panels {
            panel.scroll_offset = 0;
            // A fresh run clears the ring, so the place it was left is gone too.
            panel.upgrade_scroll_offset = 0;
        }
    }

    pub fn cycle_theme(&mut self) {
        self.theme_idx = (self.theme_idx + 1) % multitop_agent::color::THEMES.len();
    }

    #[must_use]
    pub fn current_theme(&self) -> &'static multitop_agent::color::Palette {
        &multitop_agent::color::THEMES[self.theme_idx]
    }

    pub fn rerender_all(&mut self, dims: (u16, u16)) {
        let pal = self.current_theme();
        let sort = self.sort;
        for panel in &mut self.panels {
            match panel.mode {
                Mode::Monitor | Mode::Alerts => {
                    if let Some(payload) = &panel.last_monitor {
                        let lines = crate::render_payload::render_payload(payload, dims, sort, pal);
                        panel.show_frame(lines);
                    }
                }
                Mode::Graphs => {
                    // A resize changes how many samples fit, so the graph is
                    // redrawn from the history rather than refitted -- refitting
                    // would stretch braille cells into nonsense.
                    let lines = crate::graphs::render_graphs_with_zoom(
                        &panel.history,
                        dims.0 as usize,
                        dims.1 as usize,
                        pal,
                        self.graph_zoom,
                    );
                    panel.show_frame(lines);
                }
                Mode::Docker => {
                    if let Some(payload) = &panel.last_docker {
                        let lines = crate::render_payload::render_payload(payload, dims, sort, pal);
                        panel.show_frame(lines);
                    }
                }
                Mode::Fetch => {
                    if let Some(snap) = &panel.last_fetch {
                        let lines = crate::fetch_render::render_fetch(
                            snap,
                            dims.0 as usize,
                            dims.1 as usize,
                            pal,
                        );
                        panel.show_frame(lines);
                    }
                }
                Mode::Upgrade => {}
            }
        }
    }
}
