use crate::app::App;
use crate::app::{AppMode, VaultState};
use crate::config::Server;
use crate::panel::Mode;
use crate::types::Command;
use crate::types::Msg;
use tokio::sync::mpsc::Sender;
impl App {
    #[must_use]
    pub fn upgrade_pane_header(&self, panel: usize) -> Vec<String> {
        let pal = self.current_theme();
        let Some(p) = self.panels.get(panel) else {
            return Vec::new();
        };
        let running = p.upgrade_state == crate::panel::UpgradeState::STARTED;

        let credential = if p.password_checking {
            crate::upgrade_view::Credential::Checking
        } else if p.external_password || p.password_saved {
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
            upgradable: p.upgradable.clone(),
        };

        crate::upgrade_view::header(&status, pal, Self::now_secs(), 0)
    }

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

    /// Load the passwords that can be read without touching the OS store, and
    /// report which hosts still need an asynchronous store lookup.
    ///
    /// The vault is in memory, so its copy is applied here where `&mut self`
    /// is already on the loop thread. The OS credential store is the one that
    /// can block on a system dialog; its lookup is handed back for the caller
    /// to dispatch on a blocking worker (`dispatch_credential_loads`) rather
    /// than read mid-keystroke.
    fn load_known_passwords(&mut self) -> Vec<(usize, Server)> {
        // `vault_unlocked()` rather than matching the state here: one place
        // decides what "unlocked" means, and a second copy of that match is a
        // second thing to keep in step with the state machine. The panels are
        // taken out first so the read of the vault and the write to them do not
        // overlap.
        if self.vault_unlocked().is_some() {
            let mut panels = std::mem::take(&mut self.panels);
            if let Some(vault) = self.vault_unlocked() {
                for p in &mut panels {
                    crate::vault::try_load_vault_password(p, vault);
                }
            }
            self.panels = panels;
        }
        self.panels
            .iter()
            .enumerate()
            .filter(|(_, p)| self.vault.is_none() && p.needs_credential_load())
            .map(|(i, p)| (i, p.server.clone()))
            .collect()
    }

    /// Dispatch the store lookups `load_known_passwords` reported, running each
    /// on a blocking worker so a slow or dialog-blocked keychain cannot freeze
    /// the loop the keystroke runs on. The panel is marked in-flight the moment
    /// before the worker starts, so a fast store racing the mark cannot double-
    /// dispatch.
    pub fn dispatch_credential_loads(&mut self, loads: Vec<(usize, Server)>, tx: &Sender<Msg>) {
        if loads.is_empty() {
            return;
        }
        let epoch = self.panels_epoch;
        if tokio::runtime::Handle::try_current().is_err() {
            // No runtime: the worker spawn is unavailable, so answer in place.
            // `handle_key` is only ever driven without a runtime by the direct
            // unit tests around it -- the real loop always runs the press
            // inside the runtime, where the worker branch below is the only
            // one that can fire. The load is still one-shot: `mark` happens
            // exactly as the worker would do it, so a re-dispatch cannot race
            // a second lookup.
            for (panel, server) in loads {
                let Some(p) = self.panels.get_mut(panel) else {
                    continue;
                };
                p.mark_credential_load_dispatched();
                p.answer_credential_load(crate::password_store::load(&server));
            }
            return;
        }
        for (panel, server) in loads {
            let Some(p) = self.panels.get_mut(panel) else {
                continue;
            };
            p.mark_credential_load_dispatched();
            let tx = tx.clone();
            tokio::task::spawn_blocking(move || {
                let result = crate::password_store::load(&server);
                let _ = tx.blocking_send(Msg::CredentialLoaded {
                    panel,
                    epoch,
                    result,
                });
            });
        }
    }

    /// Whether any panel's credential-store lookup is still in flight. A
    /// confirm while that is true would start a run on passwords the app has
    /// not read, so it is deferred until the last answer lands.
    #[must_use]
    pub fn any_password_checking(&self) -> bool {
        self.panels.iter().any(|p| p.password_checking)
    }

    pub fn enter_upgrade_view(&mut self) -> Vec<(usize, Server)> {
        // Do NOT reset_scroll — the user is returning to a log they may have
        // scrolled. The content is the same ring; the offset is still valid.
        // Report on credentials from what can be read silently. Passwords are
        // loaded lazily, so a panel that has not run an upgrade yet this
        // session holds nothing in memory -- and the pane read that emptiness
        // as "will prompt" for hosts whose password was saved long ago. Opening
        // this view is a deliberate user action, and telling them whether they
        // are about to be asked for a password is the point of it. The store
        // lookups this needs are dispatched by the caller; the header shows
        // `Checking` until they land.
        let loads = self.load_known_passwords();
        for p in &mut self.panels {
            // Back to where this log was left. The ring is the same one; the
            // offset into it is still meaningful, and losing it on every switch
            // was the whole complaint.
            p.scroll_offset = p.upgrade_scroll_offset;
            // Nothing else to record. The pane is composed by the renderer from
            // the ring each frame, and how much of the header stays pinned is
            // recomputed there from the header it just built -- the header
            // changes shape when a run finishes, so a count stamped here would
            // be a stale copy of a number that moves.
            p.mode = Mode::Upgrade;
        }
        loads
    }

    pub fn run_upgrade(&mut self) -> Vec<Command> {
        self.reset_scroll();
        let pal = self.current_theme();
        // Vault first, same as the view does. The store half of this loads is
        // empty in the real flow: the confirm that reaches here is gated on no
        // lookup being in flight, and enter_upgrade_view already dispatched
        // every lookup the hosts needed. A host that still needs one (a direct
        // caller that did not prime the panel) starts with none and prompts on
        // the pty, which is the fallback the store path always offered.
        let _loads = self.load_known_passwords();
        let mut cmds = Vec::new();
        for i in self.filtered_indices() {
            let gen = self.bump(i);
            let p = &mut self.panels[i];
            p.mode = Mode::Upgrade;
            if p.server.upgrade_cmd.is_some() {
                p.upgrade_state = crate::panel::UpgradeState::STARTED;
                p.upgrade_gen = gen;
                p.last_upgrade.clear();
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
            }
        }
        // Nothing is recorded per panel afterwards. The pane is composed by the
        // renderer each frame, so the skip message cannot be swallowed by
        // `ui::draw` overwriting view row 0 again: it lives in the ring, not in
        // a slot the banner owns.
        cmds
    }

    pub(crate) fn now_secs() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }

    pub fn mark_upgrades_started(&mut self, panels: &[usize]) {
        let now = Self::now_secs();
        self.upgrade_started_at = Some(now);
        // No finish time, deliberately. This is what is left on disk if the app
        // dies mid-upgrade, and it is exactly how an interrupted run is
        // detected next time.
        for &i in panels {
            let Some(p) = self.panels.get(i) else {
                continue;
            };
            if p.server.upgrade_cmd.is_none() {
                continue;
            }
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
        self.persist_state();
    }

    pub fn mark_upgrade_interrupted(&mut self, panel: usize) {
        let Some(p) = self.panels.get_mut(panel) else {
            return;
        };
        if p.upgrade_state != crate::panel::UpgradeState::STARTED {
            return;
        }
        p.upgrade_state = crate::panel::UpgradeState::DONE;
        let now = Self::now_secs();
        let key = crate::password_store::account(&p.server);
        let entry = self.host_updates.entry(key).or_default();
        entry.finished_at = Some(now);
        entry.success = false;
        self.persist_state();
    }

    pub(crate) fn persist_state(&mut self) {
        let Some(path) = self.config_path.clone() else {
            return;
        };
        let state = crate::state::AppState {
            last_update: self.last_update,
            upgrade_started_at: self.upgrade_started_at,
            hosts: self.host_updates.clone(),
            selected_host: self
                .panels
                .get(self.selected_panel)
                .map(|p| crate::password_store::account(&p.server)),
            filter_query: Some(self.filter_query.clone()).filter(|s| !s.trim().is_empty()),
            saved_filters: self.saved_filters.clone(),
            sort: Some(self.sort.word().to_string()),
            views: self
                .panels
                .iter()
                .map(|p| {
                    (
                        crate::password_store::account(&p.server),
                        // Task-backed views would restore dead (see
                        // `Mode::for_startup`): persist where the panel can
                        // actually resume, not where it happens to be.
                        p.mode.for_startup().as_str().to_string(),
                    )
                })
                .collect(),
        };
        if let Err(e) = crate::state::save_state(&path, &state) {
            let note = format!(
                "could not save upgrade state ({e}) -- an interrupted run will not be \
                 detectable after a restart."
            );
            for p in &mut self.panels {
                p.note(note.clone());
            }
        }
    }

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

    #[must_use]
    pub fn upgrade_runnable(&self) -> bool {
        self.filtered_indices()
            .iter()
            .any(|&i| self.panels[i].server.upgrade_cmd.is_some())
    }

    pub fn confirm_upgrade(&mut self) -> Vec<Command> {
        self.mode = AppMode::Running;
        let runnable = self.upgrade_runnable();
        // `run_upgrade` first, and the order is the whole point.
        //
        // It clears each started panel's `last_upgrade` ring before streaming
        // into it, and the panels are already in Upgrade mode by now -- so
        // anything `mark_upgrades_started` says goes into exactly the buffer
        // the next line empties. Marked first, a failed state write was written
        // to the pane and wiped before the frame that would have drawn it.
        let cmds = self.run_upgrade();
        if runnable {
            let shown = self.filtered_indices();
            self.mark_upgrades_started(&shown);
        }
        cmds
    }

    pub const fn cycle_banner_style(&mut self) -> crate::layout::BannerStyle {
        self.banner_style = self.banner_style.next();
        self.banner_style
    }
}
