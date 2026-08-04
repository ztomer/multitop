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
/// Only genuinely mutually exclusive UI states belong here. Quitting is a
/// terminal flag, orthogonal to what the UI is currently showing, so it lives
/// in its own field. Folding that kind of thing in here made opening a modal
/// silently discard an unrelated user setting.
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
    /// No vault exists yet and the user is choosing a master password for a new
    /// one. Reached by saving a password with no vault present, so that the
    /// vault comes into existence the first time there is something to put in
    /// it rather than needing to be set up in advance.
    Creating {
        error: Option<String>,
        /// True once Enter has handed a master password to `Vault::initialize`
        /// and the answer has not arrived yet.
        ///
        /// Argon2id at full strength takes seconds. Without this the prompt sat
        /// there with an empty field the whole time, which reads as "it did not
        /// take" -- so the password was typed and submitted again, and again,
        /// each press initialising the same vault. The second attempt then
        /// failed (the vault now existed) and its `VaultCreateFailed` carried
        /// the same epoch as the first attempt's success, so it dropped the
        /// user back onto the creation prompt with an error after the vault was
        /// already made. That is the reported "I had to enter the vault
        /// password three times".
        in_flight: bool,
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
    pub upgrade_history_lines: usize,
    /// How the panel banner draws the host name. A user preference under
    /// Appearance, never a detection: nothing here can see the terminal's font.
    pub banner_style: crate::layout::BannerStyle,
    pub password_manager: Option<crate::passwords::PasswordManager>,
    pub last_update: Option<u64>,
    pub upgrade_started_at: Option<u64>,
    pub vault: Option<Arc<multitop_vault::Vault>>,
    pub vault_state: VaultState,
    pub vault_password_input: String,
    /// Last-wins token for asynchronous vault operations.
    ///
    /// Biometric prompts and Argon2 verifications run off-thread and cannot be
    /// aborted, so cancelling one only stops the app waiting for it -- the task
    /// still finishes and still sends its result. Bumping this on every start
    /// and every cancel lets a stale result be discarded, instead of unlocking
    /// the vault and opening the upgrade modal after the user backed out.
    pub vault_epoch: u64,
    /// Per-host upgrade history, keyed by `password_store::account`. Shown in
    /// each panel's Upgrade view so the user can see what a host did last time
    /// before deciding to run it again.
    pub host_updates: std::collections::BTreeMap<String, crate::state::HostUpdate>,
    /// Terminal flag, independent of `mode`.
    pub should_quit: bool,
    /// A quit is armed but not confirmed: upgrades are in flight, and the first
    /// `q`/Esc/Ctrl-C asked rather than killed. `q` again quits; Esc stands
    /// down. Without this, the one key an operator presses to back out of a
    /// screen was also the one that killed a live `apt upgrade` on N servers.
    pub quit_armed: bool,
    /// Which incarnation of the panel *list* is current.
    ///
    /// Advanced only by `replace_panels`. Long-lived tasks that captured a panel
    /// index stamp their messages with the epoch they were started for, so an
    /// edit to the server list retires them without disturbing the per-panel
    /// `gen` that mode switches rely on.
    pub panels_epoch: u64,
}

/// Lines of headroom before a capped log is trimmed.
///
/// The upgrade log lives in a `RingLines` now and never drains, so this exists
/// only for the *visible pane* of the one-shot views (fetch/docker error
/// output), whose per-run output is small anyway. Draining only when the buffer
/// is this far over cap makes each append amortised O(1) while keeping the
/// buffer within a bounded band of its cap.
pub const LOG_AMORTIZE: usize = 512;

/// Push a line onto a capped log, amortising the trim.
///
/// `cap` is the target maximum length; the buffer may sit up to `LOG_AMORTIZE`
/// over it between trims. The single caller-class is the visible pane for
/// one-shot view output; the upgrade log uses `RingLines` and does not drain.
fn push_capped(log: &mut Vec<String>, line: String, cap: usize) {
    log.push(line);
    if log.len() > cap + LOG_AMORTIZE {
        log.drain(..log.len() - cap);
    }
}

impl App {
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
        }
    }

    /// Swap in a new panel list, invalidating every task bound to the old one.
    ///
    /// Editing the server list replaces the panels but cannot stop the monitor
    /// tasks already running: each captured its index at spawn and loops
    /// forever. Without a generation bump those tasks keep matching, so after a
    /// deletion the task for the removed host paints the panel that moved into
    /// its slot -- one machine's statistics under another machine's name.
    ///
    /// Generations continue upward from the highest one in use rather than
    /// restarting, so a task holding an old value can never coincide with a new
    /// panel.
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
    pub fn had_upgrade(&self) -> bool {
        self.panels
            .iter()
            .any(|p| p.upgrade_state != crate::panel::UpgradeState::NIL)
    }

    /// Hosts that an upgrade will skip because no `upgrade_cmd` is configured.
    /// Surfaced in the confirm row so the user knows before running. Scoped
    /// to the filter, like the run itself: a filter that hides a host means
    /// that host is not being touched.
    #[must_use]
    pub fn upgrade_skip_hosts(&self) -> Vec<String> {
        self.filtered_indices()
            .iter()
            .filter(|&&i| self.panels[i].server.upgrade_cmd.is_none())
            .map(|&i| self.panels[i].server.host.clone())
            .collect()
    }

    /// Start unlocking a locked vault. On a locked vault this enters the
    /// awaiting-biometric state and returns the shared vault handle for the
    /// caller to attempt `unlock_biometric(false)` on; a `None` return means
    /// there is no vault to unlock (or it is already unlocked), so the caller
    /// proceeds straight to the upgrade modal.
    pub fn begin_vault_unlock(&mut self) -> Option<(Arc<multitop_vault::Vault>, u64)> {
        if self.vault.is_some() && matches!(self.vault_state, VaultState::Locked) {
            let epoch = self.bump_vault_epoch();
            self.vault_state = VaultState::Unlocking {
                awaiting_biometric: true,
            };
            self.vault_password_input.clear();
            return self.vault.clone().map(|v| (v, epoch));
        }
        None
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

    /// True while a password is being verified off the event loop.
    ///
    /// Argon2id takes real time, so the attempt runs on a blocking thread and
    /// the UI shows this instead of freezing.
    #[must_use]
    pub const fn vault_verifying(&self) -> bool {
        matches!(
            self.vault_state,
            VaultState::Unlocking {
                awaiting_biometric: false
            }
        )
    }

    /// Enter the "verifying password" state.
    pub fn set_vault_unlocking(&mut self) -> u64 {
        let epoch = self.bump_vault_epoch();
        self.vault_state = VaultState::Unlocking {
            awaiting_biometric: false,
        };
        self.vault_password_input.clear();
        epoch
    }

    /// Give up on an in-flight biometric attempt.
    ///
    /// The UI blocks input while awaiting a biometric result, so without this
    /// a task that never reports leaves the app unusable -- not even quit works.
    pub fn cancel_vault_biometric(&mut self) {
        if self.vault_awaiting_biometric() {
            self.bump_vault_epoch();
            self.vault_state = VaultState::Locked;
            self.vault_password_input.clear();
        }
    }

    /// Invalidate any in-flight vault operation and return the new token.
    pub fn bump_vault_epoch(&mut self) -> u64 {
        self.vault_epoch = self.vault_epoch.wrapping_add(1);
        self.vault_epoch
    }

    /// True when a result belongs to the operation currently in force.
    #[must_use]
    pub const fn vault_epoch_current(&self, epoch: u64) -> bool {
        self.vault_epoch == epoch
    }

    /// Stop waiting on an in-flight password verification. The blocking task
    /// still finishes; its result is ignored because the state has moved on.
    pub fn cancel_vault_verify(&mut self) {
        if self.vault_verifying() {
            self.bump_vault_epoch();
            self.vault_state = VaultState::Locked;
        }
    }

    /// Decline vault creation, including one already being created off-thread.
    ///
    /// Enter spawns the creation and the prompt stays up, so Esc can land while
    /// the work is in flight. Without bumping the token, the late `VaultCreated`
    /// matched and the vault was created and seeded after the user declined.
    pub fn cancel_vault_creation(&mut self) {
        if self.vault_creating() {
            self.bump_vault_epoch();
            self.vault_state = VaultState::Locked;
            self.vault_password_input.clear();
        }
    }

    /// True while the user is choosing a master password for a brand new vault.
    #[must_use]
    pub const fn vault_creating(&self) -> bool {
        matches!(self.vault_state, VaultState::Creating { .. })
    }

    /// The error to show on the vault-creation prompt, if the last attempt
    /// failed.
    #[must_use]
    pub const fn vault_create_error(&self) -> Option<&String> {
        match &self.vault_state {
            VaultState::Creating { error, .. } => error.as_ref(),
            _ => None,
        }
    }

    /// True while a vault is being created off-thread.
    #[must_use]
    pub const fn vault_create_in_flight(&self) -> bool {
        matches!(
            self.vault_state,
            VaultState::Creating {
                in_flight: true,
                ..
            }
        )
    }

    /// Take the typed master password and mark the creation as in flight.
    ///
    /// Returns `None` when there is nothing to submit -- an empty field, or an
    /// attempt already running. One press, one attempt: the caller cannot start
    /// a second `initialize` over the first.
    pub fn begin_vault_create_attempt(&mut self) -> Option<String> {
        if self.vault_create_in_flight() {
            return None;
        }
        let master = std::mem::take(&mut self.vault_password_input);
        if master.is_empty() {
            self.vault_state = VaultState::Creating {
                error: Some("Master password cannot be empty".into()),
                in_flight: false,
            };
            return None;
        }
        self.vault_state = VaultState::Creating {
            error: None,
            in_flight: true,
        };
        Some(master)
    }

    /// Report a failed creation attempt back onto the prompt.
    pub fn fail_vault_creation(&mut self, error: String) {
        self.vault_state = VaultState::Creating {
            error: Some(error),
            in_flight: false,
        };
    }

    /// Ask for a master password so a vault can be created.
    ///
    /// Called when a password is saved and no vault exists. Returns false when
    /// a vault is already present (nothing to create) or there is no config
    /// path to put one beside.
    pub fn begin_vault_creation(&mut self) -> bool {
        if self.vault.is_some() || self.config_path.is_none() {
            return false;
        }
        self.bump_vault_epoch();
        self.vault_password_input.clear();
        self.vault_state = VaultState::Creating {
            error: None,
            in_flight: false,
        };
        true
    }

    /// Report how a master-password rotation ended, and let another one start.
    ///
    /// The outcome goes to the panels too when Server Settings has been closed
    /// in the meantime. Whether the password that unlocks the vault changed is
    /// not something the user may find out only if they happened to still be on
    /// the right screen.
    fn report_rotation(&mut self, outcome: String) {
        if let Some(manager) = self.password_manager.as_mut() {
            manager.rotating = false;
            manager.notice = Some(outcome);
            return;
        }
        for p in &mut self.panels {
            p.note(outcome.clone());
        }
    }

    /// Where a new vault file belongs: beside the config file.
    #[must_use]
    pub fn vault_path(&self) -> Option<std::path::PathBuf> {
        Some(self.config_path.as_ref()?.parent()?.join("vault.bin"))
    }

    /// Copy every password this session already knows into the freshly created
    /// vault, so the vault is useful immediately rather than only for passwords
    /// saved from now on.
    fn seed_vault_from_panels(&mut self) {
        let known: Vec<(String, String)> = self
            .panels
            .iter()
            .filter_map(|p| {
                let pass = p.sudo_password.clone()?;
                Some((crate::password_store::account(&p.server), pass))
            })
            .collect();
        let mut failed = 0usize;
        if let VaultState::Unlocked { ref mut vault, .. } = &mut self.vault_state {
            for (key, pass) in known {
                if vault
                    .set_password(key, &secrecy::SecretString::from(pass))
                    .is_err()
                {
                    failed += 1;
                }
            }
        }
        // Say so rather than leaving the user believing the vault holds
        // passwords it never received.
        if failed > 0 {
            let note = format!(
                "\u{26a0} {failed} password(s) could not be written to the new vault; \
                 they remain in the OS credential store."
            );
            for p in &mut self.panels {
                p.note(note.clone());
            }
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

    /// The panels the grid shows, or all of them when no filter is in force.
    ///
    /// This is also the scope an upgrade acts on: the renderer, the selection
    /// clamp and the upgrade path all agree on it, so a filter that narrows the
    /// screen narrows the run too.
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
            p.pinned_lines = 1;
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
            p.pinned_lines = 1;
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
            // `last_upgrade` is maintained directly by the AuxLine/AuxDone
            // handlers, so it needs no snapshot here. Copying the view into it
            // used to be the only thing preserving the completion marker, and
            // it would now also copy the pane's status header into the log and
            // duplicate it on the way back in.
            p.mode = Mode::Monitor;
            p.pinned_lines = 1;
            p.show_last_frame();
        }
        Vec::new()
    }

    /// The status header for one panel's Upgrade pane.
    ///
    /// The renderer composes this over the panel's `last_upgrade` ring on every
    /// frame, so the pane is never mirrored into `view` and the log is not
    /// cloned once per line. The header is always present — before, during and
    /// after a run — so the pane has one shape and `u` always means the same
    /// thing in it. The returned `usize` is the pinned count: the block that
    /// must not scroll away while the output under it does.
    #[must_use]
    pub fn upgrade_pane_header(&self, panel: usize) -> (Vec<String>, usize) {
        let pal = self.current_theme();
        let Some(p) = self.panels.get(panel) else {
            return (Vec::new(), 1);
        };
        let running = p.upgrade_state == crate::panel::UpgradeState::STARTED;

        let credential = if p.external_password || p.password_saved {
            crate::upgrade_view::Credential::Stored
        } else if p.sudo_password.is_some() {
            crate::upgrade_view::Credential::Session
        } else if self.vault.is_some() {
            // A locked vault has not been asked yet, and asking is what `u`
            // does. Claiming "will prompt" here was a guess dressed as a fact.
            if matches!(self.vault_state, VaultState::Unlocked { .. }) {
                crate::upgrade_view::Credential::Missing
            } else {
                crate::upgrade_view::Credential::VaultLocked
            }
        } else {
            // No vault at all: every host will prompt separately, which is the
            // difference between one prompt and one per server.
            crate::upgrade_view::Credential::MissingNoVault
        };

        let status = crate::upgrade_view::Status {
            server: &p.server,
            record: self.host_update(panel),
            credential,
            running,
        };

        let header = crate::upgrade_view::header(&status, pal, Self::now_secs(), 0);
        let pinned = header.len();
        (header, pinned)
    }

    /// How many lines a panel's pane can scroll through, whatever view it is in.
    ///
    /// The Upgrade pane is composed at draw time from the status header and the
    /// ring, so its scrollable length is those two -- not the `view` the other
    /// modes keep materialised, which the Upgrade pane does not draw at all.
    ///
    /// The header length is recomputed rather than read from `pinned_lines`:
    /// that field is stamped when the view is entered and the header changes
    /// shape when a run finishes, so it is a second copy of a number that
    /// moves.
    fn pane_len(&self, panel: usize) -> usize {
        let Some(p) = self.panels.get(panel) else {
            return 0;
        };
        if p.mode == Mode::Upgrade {
            let (header, _) = self.upgrade_pane_header(panel);
            header.len().saturating_add(p.last_upgrade.len())
        } else {
            p.view.len()
        }
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
            if p.last_upgrade.last() != Some(&note) {
                p.last_upgrade.push(note.clone());
            }
        }
    }

    /// Fill in every password this session can reach *without* asking anyone
    /// for anything.
    ///
    /// The order is the whole point. Once a vault exists it is where passwords
    /// live -- `PasswordAction::Save` writes to both stores, so the credential
    /// store holds nothing the unlocked vault does not. Reading it anyway costs
    /// a macOS keychain-access dialog per host (the binary's code signature
    /// changes on every rebuild, so the grant never sticks), and that dialog is
    /// a *second* password prompt on top of the vault unlock. Hence: vault when
    /// there is one, credential store only when there is not.
    fn load_known_passwords(&mut self) {
        if let VaultState::Unlocked { ref vault, .. } = self.vault_state {
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
        if self.vault.is_none() {
            for p in &mut self.panels {
                p.ensure_sudo_password();
            }
        }
    }

    /// First `u`: put every panel into the Upgrade view without starting
    /// anything. This is the screen the user reads before deciding to press
    /// `u` again, so it must have no side effects.
    pub fn enter_upgrade_view(&mut self) {
        self.reset_scroll();
        // Report on credentials from what can be read silently. Passwords are
        // loaded lazily, so a panel that has not run an upgrade yet this
        // session holds nothing in memory -- and the pane read that emptiness
        // as "will prompt" for hosts whose password was saved long ago. Opening
        // this view is a deliberate user action, and telling them whether they
        // are about to be asked for a password is the point of it.
        self.load_known_passwords();
        for i in 0..self.panels.len() {
            self.panels[i].mode = Mode::Upgrade;
            // The pane itself is composed by the renderer from the ring each
            // frame; all the view-switch needs to record is how much of the
            // header must stay pinned while the log scrolls under it.
            let (_, pinned) = self.upgrade_pane_header(i);
            self.panels[i].pinned_lines = pinned;
        }
    }

    /// `u`: run each server's configured upgrade command.
    ///
    /// Iterates the *filtered* set, not every panel. The run must agree with
    /// the screen: with a filter in force the hidden hosts used to be upgraded
    /// anyway while their output and failures never rendered -- "all servers"
    /// became a set the operator could not see. What you see is what you run.
    pub fn run_upgrade(&mut self) -> Vec<Command> {
        self.reset_scroll();
        let pal = self.current_theme();
        // Vault first, same as the view does.
        self.load_known_passwords();
        let mut cmds = Vec::new();
        let mut started = Vec::new();
        let mut skipped = Vec::new();
        for i in self.filtered_indices() {
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
                p.last_upgrade.replace_with(std::iter::once(format!(
                    "{}No upgrade_cmd configured for {} \u{2014} skipped{}",
                    pal.meter_high(),
                    p.server.host,
                    pal.reset
                )));
                skipped.push(i);
            }
        }

        // Record the pinned-header count for both kinds of panel. Done after
        // the loop because building the header needs `&self` while the loop
        // holds `&mut self.panels`. The pane itself is composed by the renderer
        // each frame, so the skip message can never be swallowed by `ui::draw`
        // overwriting view row 0 again: it lives in the ring, not in a slot the
        // banner owns.
        for i in started.into_iter().chain(skipped) {
            let (_, pinned) = self.upgrade_pane_header(i);
            self.panels[i].pinned_lines = pinned;
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

    /// True when at least one host *in the current filter scope* has an
    /// `upgrade_cmd` to run. With none, an upgrade could only skip every panel,
    /// so there is nothing to confirm.
    #[must_use]
    pub fn upgrade_runnable(&self) -> bool {
        self.filtered_indices()
            .iter()
            .any(|&i| self.panels[i].server.upgrade_cmd.is_some())
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
            for i in self.filtered_indices() {
                if self.panels[i].server.upgrade_cmd.is_some() {
                    let key = crate::password_store::account(&self.panels[i].server);
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

    /// Move to the next banner style and report the one now in force.
    pub const fn cycle_banner_style(&mut self) -> crate::layout::BannerStyle {
        self.banner_style = self.banner_style.next();
        self.banner_style
    }

    pub const fn quit(&mut self) {
        self.should_quit = true;
    }

    /// Whether a quit is armed but waiting for confirmation.
    #[must_use]
    pub const fn quit_armed(&self) -> bool {
        self.quit_armed
    }

    /// Stand the pending-quit confirmation down without quitting.
    pub const fn cancel_quit(&mut self) {
        self.quit_armed = false;
    }

    /// The `q`/Esc/Ctrl-C handler. When upgrades are in flight the first press
    /// arms a confirmation instead of quitting -- `Esc` is the key an operator
    /// presses to back out of a screen, and it used to kill a live `apt
    /// upgrade` on every host with a dpkg transaction mid-flight. A second
    /// `q` confirms.
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

    /// A previous run started and no completion was ever recorded after it.
    ///
    /// `upgrade_started_at` is stamped when a run begins and cleared when one
    /// finishes, so it outliving `last_update` means the app went away with a
    /// package transaction open on at least one host. The confirmation says so
    /// before starting another one -- it is the only place on the screen that
    /// carries this, and it survived the modal's deletion by being put back on
    /// the row that replaced it.
    #[must_use]
    pub fn previous_upgrade_interrupted(&self) -> bool {
        self.upgrade_started_at
            .is_some_and(|started| self.last_update.is_none_or(|last| started > last))
    }

    /// The hosts a quit would kill: those with an upgrade still in flight.
    #[must_use]
    pub fn running_upgrade_hosts(&self) -> Vec<String> {
        self.panels
            .iter()
            .filter(|p| p.upgrade_state == crate::panel::UpgradeState::STARTED)
            .map(|p| p.server.host.clone())
            .collect()
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
    pub fn apply(&mut self, msg: Msg) -> bool {
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
                    return false;
                };

                match &payload {
                    // The banner host name is drawn on every panel whatever
                    // view it is in, so a Monitor packet always changes what is
                    // on screen.
                    multitop_agent::proto::Payload::Monitor(_) => {
                        p.last_monitor = Some(payload.clone());
                        let lines =
                            crate::render_payload::render_payload(&payload, dims, sort, pal);
                        p.last_frame = Some(lines.clone());
                        if p.mode == Mode::Monitor {
                            p.view = lines;
                        }
                        true
                    }
                    multitop_agent::proto::Payload::Docker { .. } => {
                        p.last_docker = Some(payload.clone());
                        let shown = p.mode == Mode::Docker && accepts;
                        if shown {
                            let lines =
                                crate::render_payload::render_payload(&payload, dims, sort, pal);
                            p.view = lines;
                        }
                        shown
                    }
                    multitop_agent::proto::Payload::Fetch(snap) => {
                        p.last_fetch = Some(snap.clone());
                        let shown = p.mode == Mode::Fetch && accepts;
                        if shown {
                            let lines = crate::fetch_render::render_fetch(
                                snap,
                                dims.0 as usize,
                                dims.1 as usize,
                                pal,
                            );
                            p.view = lines;
                        }
                        shown
                    }
                }
            }
            Msg::Frame {
                panel,
                epoch,
                lines,
            } => {
                if epoch != self.panels_epoch {
                    return false;
                }
                let Some(p) = self.panels.get_mut(panel) else {
                    return false;
                };
                p.last_frame = Some(lines);
                // Only paint it if stats is what the user is looking at.
                if p.mode == Mode::Monitor {
                    p.show_last_frame();
                    true
                } else {
                    false
                }
            }
            Msg::Status { panel, gen, text } => {
                if self.accepts(panel, gen) {
                    let p = &mut self.panels[panel];
                    if p.mode == Mode::Upgrade {
                        // In the Upgrade view a status note is one more line in
                        // the log. Replacing the whole view here wiped the
                        // status header *and* every line of output collected so
                        // far, which is what left panels showing nothing but
                        // "sudo ready" in the middle of a run.
                        p.last_upgrade.push(text);
                    } else {
                        p.view = vec![text];
                    }
                    true
                } else {
                    false
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
                    true
                } else {
                    false
                }
            }
            Msg::AuxBegin { panel, gen, header } => {
                if self.accepts(panel, gen) {
                    let p = &mut self.panels[panel];
                    // In the Upgrade view the pane already has its status header
                    // and is about to receive output; replacing the whole view
                    // here threw that away on every single run, leaving nothing
                    // but a bare "Upgrade on <host>" line that the panel banner
                    // then overwrote. Other views use this as their reset.
                    if p.mode == Mode::Upgrade {
                        false
                    } else {
                        p.view = header.into_iter().collect();
                        true
                    }
                } else {
                    false
                }
            }
            Msg::AuxLine { panel, gen, line } => {
                let cap = self.upgrade_history_lines;
                let Some(p) = self.panels.get_mut(panel) else {
                    return false;
                };
                // `last_upgrade` is the durable log for this panel's upgrade. It
                // is keyed on `upgrade_gen`, not `gen`, so it keeps filling while
                // the user is looking at another view. Pushing into the ring
                // reuses the oldest slot's allocation in place -- no clone, no
                // shift -- and the pane renders from it, so there is no separate
                // view copy to keep in sync.
                let belongs =
                    p.upgrade_state == crate::panel::UpgradeState::STARTED && p.upgrade_gen == gen;
                let visible = if belongs {
                    p.mode == Mode::Upgrade
                } else {
                    p.gen == gen
                };
                if belongs {
                    p.last_upgrade.push(line);
                } else if p.gen == gen {
                    // Output for the current view's own run (fetch/docker error
                    // lines): it belongs to the visible pane only.
                    push_capped(&mut p.view, line, cap);
                }
                visible
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
                    return false;
                }
                let cap = self.upgrade_history_lines;
                // Captured before the state flips to DONE below.
                let belongs = self.panels[panel].upgrade_state
                    == crate::panel::UpgradeState::STARTED
                    && self.panels[panel].upgrade_gen == gen;
                if belongs {
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
                    let p = &mut self.panels[panel];
                    // The completion marker belongs in the durable log too. It
                    // used to go only into `view`, so finishing while the user
                    // was on another view appended it to *that* view and lost it
                    // from the upgrade output for good.
                    if belongs {
                        p.last_upgrade.push(note);
                    } else if p.gen == gen {
                        push_capped(&mut p.view, note, cap);
                    }
                }
                // The pane's status header is recomposed by the renderer from
                // `upgrade_state` every frame, so a finished run stops
                // advertising itself as running without a rebuild here.
                true
            }
            Msg::VaultCreated { epoch, unlocked } => {
                if !self.vault_epoch_current(epoch) {
                    return false;
                }
                if let Some(ref path) = self.config_path {
                    self.vault = crate::vault::create_vault(path).map(Arc::new);
                }
                self.vault_state = VaultState::Unlocked {
                    vault: unlocked,
                    awaiting_biometric: false,
                };
                self.seed_vault_from_panels();
                self.vault_password_input.clear();
                let note = "Vault created. Sudo passwords are now stored encrypted; \
                     unlock with Touch ID.";
                // Said where the user is looking. Creating a vault starts in
                // Server Settings, whose panel covers the whole screen, so a
                // line appended to the panels behind it is a line nobody reads.
                if let Some(manager) = self.password_manager.as_mut() {
                    manager.notice = Some(note.to_string());
                }
                for p in &mut self.panels {
                    p.note(note.to_string());
                }
                true
            }
            Msg::VaultUnlockFailed { epoch, error } => {
                if !self.vault_epoch_current(epoch) {
                    return false;
                }
                // Back to the prompt with the reason, rather than silently
                // dropping the user somewhere with no explanation.
                self.vault_state = VaultState::PasswordPrompt { error: Some(error) };
                true
            }
            Msg::VaultCreateFailed { epoch, error } => {
                // Also refuses to reopen the prompt over a vault that exists:
                // the failing attempt may be a duplicate of one that already
                // succeeded, and reporting it would take a working vault back
                // off the user.
                if !self.vault_epoch_current(epoch) || self.vault.is_some() {
                    return false;
                }
                self.fail_vault_creation(error);
                true
            }
            Msg::VaultUnlocked { epoch, unlocked } => {
                if !self.vault_epoch_current(epoch) {
                    return false;
                }
                self.vault_state = VaultState::Unlocked {
                    vault: unlocked,
                    awaiting_biometric: false,
                };
                self.mode = AppMode::ShowUpgradeModal;
                true
            }
            Msg::VaultPasswordRotated { epoch } => {
                if !self.vault_epoch_current(epoch) {
                    return false;
                }
                // The vault key is unchanged by a rotation, so an unlocked
                // handle stays valid and any Secure Enclave wrapper still
                // decrypts. Only the password that unwraps it has moved.
                self.report_rotation("Master password changed.".to_string());
                true
            }
            Msg::VaultPasswordRotationFailed { epoch, error } => {
                if !self.vault_epoch_current(epoch) {
                    return false;
                }
                // Said plainly, because the common cause is a mistyped current
                // password and the useful fact is that nothing changed.
                self.report_rotation(format!("Master password NOT changed: {error}"));
                true
            }
            Msg::VaultBiometricFailed { epoch } => {
                if !self.vault_epoch_current(epoch) {
                    return false;
                }
                // Biometrics unavailable or cancelled: fall back to the password
                // prompt. `Unlocking { awaiting_biometric: false }` would be a
                // dead end — no prompt, no modal, nothing for the user to do.
                self.vault_state = VaultState::PasswordPrompt { error: None };
                self.vault_password_input.clear();
                true
            }
        }
    }

    pub fn scroll_up(&mut self, delta: usize) {
        if self.selected_panel < self.panels.len() {
            let max_scroll = self.pane_len(self.selected_panel).saturating_sub(1);
            let p = &mut self.panels[self.selected_panel];
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
            let max_scroll = self.pane_len(panel).saturating_sub(1);
            let p = &mut self.panels[panel];
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
