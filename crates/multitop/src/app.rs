//! Application state.
//!
//! Every state transition here is a pure function of the current state plus
//! one message, and returns the side effects it wants performed. The async
//! runtime in `run.rs` does the I/O; this module can be tested without a
//! terminal or a network.

#![allow(clippy::missing_const_for_fn)]

use secrecy::ExposeSecret;
use std::sync::Arc;

use crate::config::Server;

pub use crate::panel::{Mode, Panel};
pub use crate::types::{Command, Msg};

pub use multitop_agent::SortBy;

/// High-level application mode.
///
/// Only genuinely mutually exclusive UI states belong here. Sparkline
/// visibility is a persisted user preference and quitting is a terminal flag —
/// both are orthogonal to what the UI is currently showing, so they live in
/// their own fields. Folding them in made opening a modal silently discard the
/// user's sparkline setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AppMode {
    #[default]
    Running,
    Filtering,
    ShowUpgradeModal,
}

/// Vault authentication state machine.
#[derive(Debug, Default)]
pub enum VaultState {
    #[default]
    Locked,
    Unlocking {
        awaiting_biometric: bool,
    },
    Unlocked {
        vault: Box<multitop_vault::UnlockedVault>,
        awaiting_biometric: bool,
    },
    PasswordPrompt {
        error: Option<String>,
    },
}

pub struct App {
    pub panels: Vec<Panel>,
    pub selected_panel: usize,
    pub mode: AppMode,
    pub sort: SortBy,
    pub theme_idx: usize,
    pub config_path: Option<std::path::PathBuf>,
    pub filter_query: String,
    pub sparklines: Vec<crate::sparkline::SparklineHistory>,
    pub sparklines_mem: Vec<crate::sparkline::SparklineHistory>,
    pub sparklines_cpu: Vec<crate::sparkline::SparklineHistory>,
    pub upgrade_history_lines: usize,
    pub password_manager: Option<crate::passwords::PasswordManager>,
    pub last_update: Option<u64>,
    pub upgrade_started_at: Option<u64>,
    pub vault: Option<Arc<multitop_vault::Vault>>,
    pub vault_state: VaultState,
    pub vault_password_input: String,
    /// Per-host upgrade history, keyed by `password_store::account`. Shown in
    /// each panel's Upgrade view so the user can see what a host did last time
    /// before deciding to run it again.
    pub host_updates: std::collections::BTreeMap<String, crate::state::HostUpdate>,
    /// Persisted user preference, independent of `mode`.
    pub show_sparklines: bool,
    /// Terminal flag, independent of `mode`.
    pub should_quit: bool,
}

impl App {
    pub fn new(servers: Vec<Server>) -> Self {
        let count = servers.len();
        Self {
            panels: servers.into_iter().map(Panel::new).collect(),
            selected_panel: 0,
            mode: AppMode::Running,
            sort: SortBy::Cpu,
            theme_idx: 0,
            config_path: None,
            filter_query: String::new(),
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
            last_update: None,
            upgrade_started_at: None,
            vault: None,
            vault_state: VaultState::default(),
            vault_password_input: String::new(),
            host_updates: std::collections::BTreeMap::new(),
            show_sparklines: false,
            should_quit: false,
        }
    }

    #[must_use]
    pub fn upgrades_in_flight(&self) -> bool {
        self.panels
            .iter()
            .any(|p| p.upgrade_state == crate::panel::UpgradeState::STARTED)
    }

    #[must_use]
    pub fn had_upgrade(&self) -> bool {
        self.panels
            .iter()
            .any(|p| p.upgrade_state != crate::panel::UpgradeState::NIL)
    }

    /// Hosts that an upgrade will skip because no `upgrade_cmd` is configured.
    /// Surfaced in the confirm modal so the user knows before running.
    #[must_use]
    pub fn upgrade_skip_hosts(&self) -> Vec<String> {
        self.panels
            .iter()
            .filter(|p| p.server.upgrade_cmd.is_none())
            .map(|p| p.server.host.clone())
            .collect()
    }

    /// Start unlocking a locked vault. On a locked vault this enters the
    /// awaiting-biometric state and returns the shared vault handle for the
    /// caller to attempt `unlock_biometric(false)` on; a `None` return means
    /// there is no vault to unlock (or it is already unlocked), so the caller
    /// proceeds straight to the upgrade modal.
    pub fn begin_vault_unlock(&mut self) -> Option<Arc<multitop_vault::Vault>> {
        if self.vault.is_some() && matches!(self.vault_state, VaultState::Locked) {
            self.vault_state = VaultState::Unlocking {
                awaiting_biometric: true,
            };
            self.vault_password_input.clear();
            return self.vault.clone();
        }
        None
    }

    /// Check if sparklines should be shown.
    #[must_use]
    pub const fn show_sparklines(&self) -> bool {
        self.show_sparklines
    }

    /// Toggle sparklines visibility.
    pub const fn toggle_sparklines(&mut self) {
        self.show_sparklines = !self.show_sparklines;
    }

    /// Check if upgrade modal should be shown.
    #[must_use]
    pub const fn show_upgrade_modal(&self) -> bool {
        matches!(self.mode, AppMode::ShowUpgradeModal)
    }

    /// Set upgrade modal visibility.
    pub fn set_show_upgrade_modal(&mut self, show: bool) {
        if show {
            self.mode = AppMode::ShowUpgradeModal;
        } else if matches!(self.mode, AppMode::ShowUpgradeModal) {
            self.mode = AppMode::Running;
        }
    }

    /// Check if vault password prompt should be shown.
    #[must_use]
    pub const fn show_vault_password_prompt(&self) -> bool {
        matches!(self.vault_state, VaultState::PasswordPrompt { .. })
    }

    /// Set vault password prompt visibility.
    ///
    /// Showing the prompt is idempotent: re-asserting it while already
    /// prompting keeps any error already on display, so callers that set the
    /// error and then (re)open the prompt do not silently discard it.
    pub fn set_show_vault_password_prompt(&mut self, show: bool) {
        match (show, &self.vault_state) {
            (true, VaultState::PasswordPrompt { .. }) => {}
            (true, _) => self.vault_state = VaultState::PasswordPrompt { error: None },
            (false, VaultState::PasswordPrompt { .. }) => self.vault_state = VaultState::Locked,
            (false, _) => {}
        }
    }

    /// Check if vault is awaiting biometric authentication.
    #[must_use]
    pub const fn vault_awaiting_biometric(&self) -> bool {
        matches!(
            self.vault_state,
            VaultState::Unlocking {
                awaiting_biometric: true
            } | VaultState::Unlocked {
                awaiting_biometric: true,
                ..
            }
        )
    }

    /// Get vault password input.
    #[must_use]
    pub fn vault_password_input(&self) -> &str {
        &self.vault_password_input
    }

    /// Get mutable vault password input.
    pub fn vault_password_input_mut(&mut self) -> &mut String {
        &mut self.vault_password_input
    }

    /// Get vault password error.
    #[must_use]
    pub const fn vault_password_error(&self) -> Option<&String> {
        match &self.vault_state {
            VaultState::PasswordPrompt { error } => error.as_ref(),
            _ => None,
        }
    }

    /// Set vault password error.
    pub fn set_vault_password_error(&mut self, err: Option<String>) {
        if let VaultState::PasswordPrompt { ref mut error } = &mut self.vault_state {
            *error = err;
        }
    }

    /// Get the unlocked vault if available.
    #[must_use]
    pub const fn vault_unlocked(&self) -> Option<&multitop_vault::UnlockedVault> {
        match &self.vault_state {
            VaultState::Unlocked { vault, .. } => Some(vault),
            _ => None,
        }
    }

    /// Get mutable unlocked vault if available.
    #[must_use]
    pub fn vault_unlocked_mut(&mut self) -> Option<&mut multitop_vault::UnlockedVault> {
        match &mut self.vault_state {
            VaultState::Unlocked { vault, .. } => Some(vault),
            _ => None,
        }
    }

    /// Check if should quit.
    #[must_use]
    pub const fn should_quit(&self) -> bool {
        self.should_quit
    }

    /// Check if filtering.
    #[must_use]
    pub const fn is_filtering(&self) -> bool {
        matches!(self.mode, AppMode::Filtering)
    }

    /// Set filtering mode.
    pub fn set_filtering(&mut self, filtering: bool) {
        if filtering {
            self.mode = AppMode::Filtering;
        } else if matches!(self.mode, AppMode::Filtering) {
            self.mode = AppMode::Running;
        }
    }

    #[must_use]
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

    #[must_use]
    pub fn in_docker(&self) -> bool {
        self.panels.iter().any(|p| p.mode == Mode::Docker)
    }

    #[must_use]
    pub fn in_fetch(&self) -> bool {
        self.panels.iter().any(|p| p.mode == Mode::Fetch)
    }

    #[must_use]
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
        for i in 0..self.panels.len() {
            self.panels[i].mode = Mode::Upgrade;
            let view = self.upgrade_pane(i, false);
            self.panels[i].view = view;
        }
    }

    /// The Upgrade pane for one panel: a status header, then whatever output
    /// the last run produced.
    ///
    /// The header is always present — before, during and after a run — so the
    /// pane has one shape and `u` always means the same thing in it.
    fn upgrade_pane(&self, panel: usize, running: bool) -> Vec<String> {
        let pal = self.current_theme();
        let Some(p) = self.panels.get(panel) else {
            return Vec::new();
        };

        let credential = if p.external_password || p.password_saved {
            crate::upgrade_view::Credential::Stored
        } else if p.sudo_password.is_some() {
            crate::upgrade_view::Credential::Session
        } else {
            crate::upgrade_view::Credential::Missing
        };

        let status = crate::upgrade_view::Status {
            server: &p.server,
            record: self.host_update(panel),
            credential,
            running,
        };

        let mut out = crate::upgrade_view::header(&status, pal, Self::now_secs(), 0);
        out.extend(p.last_upgrade.iter().cloned());
        out
    }

    /// Second `u` with no host configured to upgrade: there is nothing to
    /// confirm, so explain that in the pane rather than opening a modal whose
    /// only possible outcome is skipping everything.
    pub fn note_nothing_to_upgrade(&mut self) {
        let pal = self.current_theme();
        let note = format!(
            "{}\u{26a0} No host has an upgrade_cmd \u{2014} nothing to run.{}",
            pal.meter_high(),
            pal.reset
        );
        for p in &mut self.panels {
            if p.view.last() != Some(&note) {
                p.view.push(note.clone());
            }
        }
    }

    /// First `u`: put every panel into the Upgrade view without starting
    /// anything. This is the screen the user reads before deciding to press
    /// `u` again, so it must have no side effects.
    pub fn enter_upgrade_view(&mut self) {
        self.reset_scroll();
        for i in 0..self.panels.len() {
            self.panels[i].mode = Mode::Upgrade;
            let running = self.panels[i].upgrade_state == crate::panel::UpgradeState::STARTED;
            let view = self.upgrade_pane(i, running);
            self.panels[i].view = view;
        }
    }

    /// `u`: run each server's configured upgrade command.
    pub fn run_upgrade(&mut self) -> Vec<Command> {
        self.reset_scroll();
        let pal = self.current_theme();
        // Pre-load vault passwords if vault is unlocked
        if let VaultState::Unlocked { ref vault, .. } = &self.vault_state {
            for p in &mut self.panels {
                if p.sudo_password.is_none() {
                    let key = crate::password_store::account(&p.server);
                    if let Some(pass) = vault.get_password(&key) {
                        p.sudo_password = Some(pass.expose_secret().to_string());
                        p.external_password = true;
                    }
                }
            }
        }
        let mut cmds = Vec::new();
        let mut started = Vec::new();
        let mut skipped = Vec::new();
        for i in 0..self.panels.len() {
            let gen = self.bump(i);
            let p = &mut self.panels[i];
            p.mode = Mode::Upgrade;
            p.ensure_sudo_password();
            if p.server.upgrade_cmd.is_some() {
                p.upgrade_state = crate::panel::UpgradeState::STARTED;
                p.upgrade_gen = gen;
                p.last_upgrade.clear();
                started.push(i);
                cmds.push(Command::RunUpgrade { panel: i, gen });
            } else {
                // One line, naming the host. The pane header already carries the
                // "set upgrade_cmd in config.toml" guidance, so the old second
                // hint line here would just be the same advice twice in a panel
                // that may only be forty columns wide.
                p.upgrade_state = crate::panel::UpgradeState::DONE;
                p.upgrade_gen = gen;
                p.last_upgrade = vec![format!(
                    "{}No upgrade_cmd configured for {} \u{2014} skipped{}",
                    pal.meter_high(),
                    p.server.host,
                    pal.reset
                )];
                skipped.push(i);
            }
        }

        // Rebuild both kinds of panel through the same header the pane always
        // shows. Done after the loop because building the header needs `&self`
        // while the loop holds `&mut self.panels`.
        //
        // This also stops the skip message from being swallowed: it used to sit
        // at view[0], which `ui::draw` overwrites with the host banner on every
        // frame, so the user only ever saw the follow-up hint.
        for i in started {
            let view = self.upgrade_pane(i, true);
            self.panels[i].view = view;
        }
        for i in skipped {
            let view = self.upgrade_pane(i, false);
            self.panels[i].view = view;
        }
        cmds
    }

    /// Unix seconds now.
    fn now_secs() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }

    /// Write the whole runtime state, including per-host records, to disk.
    fn persist_state(&self) {
        if let Some(ref path) = self.config_path {
            let state = crate::state::AppState {
                last_update: self.last_update,
                upgrade_started_at: self.upgrade_started_at,
                hosts: self.host_updates.clone(),
            };
            let _ = crate::state::save_state(path, &state);
        }
    }

    /// The last recorded upgrade for a panel's host.
    #[must_use]
    pub fn host_update(&self, panel: usize) -> crate::state::HostUpdate {
        self.panels
            .get(panel)
            .map_or_else(crate::state::HostUpdate::default, |p| {
                self.host_updates
                    .get(&crate::password_store::account(&p.server))
                    .copied()
                    .unwrap_or_default()
            })
    }

    /// True when at least one host has an `upgrade_cmd` to run. With none, an
    /// upgrade could only skip every panel, so there is nothing to confirm.
    #[must_use]
    pub fn upgrade_runnable(&self) -> bool {
        self.panels.iter().any(|p| p.server.upgrade_cmd.is_some())
    }

    /// Confirm upgrade from modal and execute `run_upgrade`.
    pub fn confirm_upgrade(&mut self) -> Vec<Command> {
        self.mode = AppMode::Running;
        if self.upgrade_runnable() {
            let now = Self::now_secs();
            self.upgrade_started_at = Some(now);
            // Mark each runnable host as started with no finish time. If the
            // app dies mid-upgrade this is what is left on disk, and it is
            // exactly how an interrupted run is detected next time.
            for p in &self.panels {
                if p.server.upgrade_cmd.is_some() {
                    let key = crate::password_store::account(&p.server);
                    self.host_updates.insert(
                        key,
                        crate::state::HostUpdate {
                            started_at: Some(now),
                            finished_at: None,
                            success: false,
                        },
                    );
                }
            }
            self.persist_state();
        }
        self.run_upgrade()
    }

    pub const fn quit(&mut self) {
        self.should_quit = true;
    }

    /// True when a message is still relevant to the panel it targets.
    fn accepts(&self, panel: usize, gen: u64) -> bool {
        self.panels.get(panel).is_some_and(|p| p.gen == gen)
    }

    #[allow(
        clippy::too_many_lines,
        clippy::cast_possible_truncation,
        clippy::cast_precision_loss
    )]
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
                        let lines =
                            crate::render_payload::render_payload(&payload, dims, sort, pal);
                        p.last_frame = Some(lines.clone());
                        if p.mode == Mode::Monitor {
                            p.view = lines;
                        }
                    }
                    multitop_agent::proto::Payload::Docker { .. } => {
                        p.last_docker = Some(payload.clone());
                        if p.mode == Mode::Docker && accepts {
                            let lines =
                                crate::render_payload::render_payload(&payload, dims, sort, pal);
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
                let cap = self.upgrade_history_lines;
                let Some(p) = self.panels.get_mut(panel) else {
                    return;
                };
                if p.gen == gen {
                    p.view.push(line.clone());
                    if p.view.len() > cap {
                        p.view.drain(..p.view.len() - cap);
                    }
                }
                if p.upgrade_state == crate::panel::UpgradeState::STARTED && p.upgrade_gen == gen {
                    p.last_upgrade.push(line);
                    if p.last_upgrade.len() > cap {
                        p.last_upgrade.drain(..p.last_upgrade.len() - cap);
                    }
                }
            }
            Msg::AuxDone {
                panel,
                gen,
                note,
                success,
            } => {
                if !self.accepts(panel, gen)
                    && !self.panels.get(panel).is_some_and(|p| {
                        p.upgrade_gen == gen
                            && p.upgrade_state == crate::panel::UpgradeState::STARTED
                    })
                {
                    return;
                }
                if self.panels[panel].upgrade_state == crate::panel::UpgradeState::STARTED
                    && self.panels[panel].upgrade_gen == gen
                {
                    self.panels[panel].upgrade_state = crate::panel::UpgradeState::DONE;
                    let now = Self::now_secs();

                    // Record this host's outcome regardless of how the others
                    // fared — the panel shows its own history, and a failure on
                    // one host must not erase another host's success.
                    let key = crate::password_store::account(&self.panels[panel].server);
                    let entry = self.host_updates.entry(key).or_default();
                    entry.finished_at = Some(now);
                    entry.success = success;

                    if success && !self.upgrades_in_flight() {
                        self.last_update = Some(now);
                        self.upgrade_started_at = None;
                    }
                    self.persist_state();
                }
                if let Some(note) = note {
                    self.panels[panel].view.push(note);
                }
            }
            Msg::VaultUnlocked(unlocked) => {
                self.vault_state = VaultState::Unlocked {
                    vault: Box::new(unlocked),
                    awaiting_biometric: false,
                };
                self.mode = AppMode::ShowUpgradeModal;
            }
            Msg::VaultBiometricFailed => {
                // Biometrics unavailable or cancelled: fall back to the password
                // prompt. `Unlocking { awaiting_biometric: false }` would be a
                // dead end — no prompt, no modal, nothing for the user to do.
                self.vault_state = VaultState::PasswordPrompt { error: None };
                self.vault_password_input.clear();
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
        for panel in &mut self.panels {
            panel.scroll_offset = 0;
        }
    }

    pub fn cycle_theme(&mut self) {
        self.theme_idx = (self.theme_idx + 1) % multitop_agent::color::THEMES.len();
    }

    #[must_use]
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
                        panel.view =
                            crate::render_payload::render_payload(payload, dims, sort, pal);
                    }
                }
                Mode::Docker => {
                    if let Some(payload) = &panel.last_docker {
                        panel.view =
                            crate::render_payload::render_payload(payload, dims, sort, pal);
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
