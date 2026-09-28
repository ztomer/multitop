//! Application state.

mod apply;
mod apply_vault;
mod ops_view;
mod render;
mod types;
mod upgrade;
mod vault;
mod views;

pub use crate::panel::{Mode, Panel};
pub use crate::types::{Command, Msg};
pub use types::{AppMode, Confirm, ExecConfirm, ExecKind, Overlay, VaultState};

pub struct App {
    pub panels: Vec<Panel>,
    pub selected_panel: usize,
    pub mode: AppMode,
    pub sort: multitop_agent::SortBy,
    pub theme_idx: usize,
    /// What the terminal is painting behind us, once at startup.
    ///
    /// Stored rather than queried on demand because the answer cannot change
    /// while the process runs, and a per-frame query would put a terminal round
    /// trip in the render path. `Unknown` is the honest value for a terminal that
    /// answered neither probe, and the theme's own background is the fallback.
    pub background: crate::background::Background,
    /// The palette to draw with, adapted for the background we detected.
    ///
    /// Held rather than computed on demand: adapting allocates the escapes, and
    /// a render path that allocated per frame would be worse than the bug it
    /// fixes. `boot_app` builds it once, `cycle_theme` rebuilds it.
    ///
    /// A shared handle, not a plain field, and that is not a micro-optimisation.
    /// The render path needs the palette while it holds `&mut` borrows of the
    /// panels it is drawing, so a palette reachable only by borrowing `self` is a
    /// borrow conflict at six call sites. Cloning the handle is a pointer copy,
    /// so the conflict disappears and so does any temptation to clone the twelve
    /// owned escapes instead.
    ///
    /// `Arc` rather than `Rc` because the `App` is moved onto a tokio task in the
    /// event-loop tests, and `Rc` is not `Send`. An `Rc` would compile everywhere
    /// else and fail in exactly one place, which is the worst way to find out;
    /// the atomic it costs is on a path that was already a channel send.
    pub palette_view: std::sync::Arc<multitop_agent::palette_view::PaletteView>,
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
    /// The overlay in front of the panels: help, the command palette, or none.
    pub overlay: Overlay,
    pub focused_panel: Option<usize>,
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
