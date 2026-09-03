//! Application state.

#![allow(clippy::missing_const_for_fn)]

mod apply;
mod render;
mod types;
mod upgrade;
mod vault;
mod views;

pub use crate::panel::{Mode, Panel};
pub use crate::types::{Command, Msg};
pub use types::{AppMode, Confirm, ExecConfirm, ExecKind, VaultState};

#[allow(clippy::struct_excessive_bools)]
pub struct App {
    pub panels: Vec<Panel>,
    pub selected_panel: usize,
    pub mode: AppMode,
    pub sort: multitop_agent::SortBy,
    pub theme_idx: usize,
    pub config_path: Option<std::path::PathBuf>,
    pub filter_query: String,
    pub upgrade_history_lines: usize,
    pub banner_style: crate::layout::BannerStyle,
    pub password_manager: Option<crate::passwords::PasswordManager>,
    pub last_update: Option<u64>,
    pub upgrade_started_at: Option<u64>,
    pub vault: Option<std::sync::Arc<multitop_vault::Vault>>,
    pub vault_state: VaultState,
    pub vault_password_input: String,
    pub vault_epoch: u64,
    pub host_updates: std::collections::BTreeMap<String, crate::state::HostUpdate>,
    pub should_quit: bool,
    pub quit_armed: bool,
    pub panels_epoch: u64,
    pub help_visible: bool,
    pub focused_panel: Option<usize>,
    pub command_palette_visible: bool,
    pub command_input: String,
    pub graph_zoom: u8,
    pub alert_cpu: Option<u8>,
    pub alert_mem: Option<u8>,
    pub alert_disk: Option<u8>,
    pub alert_targets: Vec<crate::config::AlertTarget>,
    pub saved_filters: Vec<String>,
    /// `x/o/r` armed for `host:pid:name` per roadmap Phase 3.
    pub kill_confirm: Option<ExecConfirm>,
}

pub const LOG_AMORTIZE: usize = 512;

pub(crate) fn push_capped(log: &mut Vec<String>, line: String, cap: usize) {
    log.push(line);
    if log.len() > cap + LOG_AMORTIZE {
        log.drain(..log.len() - cap);
    }
}
