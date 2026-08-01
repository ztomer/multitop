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

/// High-level application mode (mutually exclusive UI states).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AppMode {
    #[default]
    Running,
    Filtering,
    ShowUpgradeModal,
    ShowSparklines,
    ShouldQuit,
}

/// Vault authentication state machine.
#[derive(Debug, Default)]
pub enum VaultState {
    #[default]
    Locked,
    Unlocking { awaiting_biometric: bool },
    Unlocked { vault: Box<multitop_vault::UnlockedVault>, awaiting_biometric: bool },
    PasswordPrompt { error: Option<String> },
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
            self.vault_state = VaultState::Unlocking { awaiting_biometric: true };
            self.vault_password_input.clear();
            return self.vault.clone();
        }
        None
    }

    /// Check if sparklines should be shown.
    #[must_use]
    pub const fn show_sparklines(&self) -> bool {
        matches!(self.mode, AppMode::ShowSparklines)
    }

    /// Toggle sparklines visibility.
    pub fn toggle_sparklines(&mut self) {
        self.mode = if self.show_sparklines() {
            AppMode::Running
        } else {
            AppMode::ShowSparklines
        };
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
    pub fn set_show_vault_password_prompt(&mut self, show: bool) {
        if show {
            self.vault_state = VaultState::PasswordPrompt { error: None };
        } else if matches!(self.vault_state, VaultState::PasswordPrompt { .. }) {
            self.vault_state = VaultState::Locked;
        }
    }

    /// Check if vault is awaiting biometric authentication.
    #[must_use]
    pub const fn vault_awaiting_biometric(&self) -> bool {
        matches!(self.vault_state, VaultState::Unlocking { awaiting_biometric: true }
            | VaultState::Unlocked { awaiting_biometric: true, .. })
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
    pub fn should_quit(&self) -> bool {
        matches!(self.mode, AppMode::ShouldQuit)
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
                p.last_upgrade.clone()
            };
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
        for i in 0..self.panels.len() {
            let gen = self.bump(i);
            let p = &mut self.panels[i];
            p.mode = Mode::Upgrade;
            p.ensure_sudo_password();
            if p.server.upgrade_cmd.is_some() {
                p.upgrade_state = crate::panel::UpgradeState::STARTED;
                p.upgrade_gen = gen;
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
            } else {
                let msg = format!(
                    "{}No upgrade_cmd configured for {} — skipped{}\n",
                    pal.meter_high(),
                    p.server.host,
                    pal.reset
                );
                let hint = format!(
                    "{}Set upgrade_cmd in the config to enable updates for this server{}",
                    pal.muted(),
                    pal.reset
                );
                p.upgrade_state = crate::panel::UpgradeState::DONE;
                p.upgrade_gen = gen;
                p.last_upgrade = vec![msg.clone(), hint.clone()];
                p.view = vec![msg, hint];
            }
        }
        cmds
    }

    /// Confirm upgrade from modal and execute `run_upgrade`.
    pub fn confirm_upgrade(&mut self) -> Vec<Command> {
        self.mode = AppMode::Running;
        let has_upgrade = self.panels.iter().any(|p| p.server.upgrade_cmd.is_some());
        if has_upgrade {
            self.upgrade_started_at = Some(
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs(),
            );
            if let Some(ref path) = self.config_path {
                let state = crate::state::AppState {
                    last_update: self.last_update,
                    upgrade_started_at: self.upgrade_started_at,
                };
                let _ = crate::state::save_state(path, &state);
            }
        }
        self.run_upgrade()
    }

    pub fn quit(&mut self) {
        self.mode = AppMode::ShouldQuit;
    }

    /// True when a message is still relevant to the panel it targets.
    fn accepts(&self, panel: usize, gen: u64) -> bool {
        self.panels.get(panel).is_some_and(|p| p.gen == gen)
    }

    #[allow(clippy::too_many_lines, clippy::cast_possible_truncation, clippy::cast_precision_loss)]
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
                    if success && !self.upgrades_in_flight() {
                        self.last_update = Some(
                            std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_secs(),
                        );
                        self.upgrade_started_at = None;
                        if let Some(ref path) = self.config_path {
                            let state = crate::state::AppState {
                                last_update: self.last_update,
                                upgrade_started_at: None,
                            };
                            let _ = crate::state::save_state(path, &state);
                        }
                    }
                }
                if let Some(note) = note {
                    self.panels[panel].view.push(note);
                }
            }
            Msg::VaultUnlocked(unlocked) => {
                self.vault_state = VaultState::Unlocked { vault: Box::new(unlocked), awaiting_biometric: false };
                self.mode = AppMode::ShowUpgradeModal;
            }
            Msg::VaultBiometricFailed => {
                self.vault_state = VaultState::Unlocking { awaiting_biometric: false };
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
