//! Per-panel task handles: monitors, aux views (fetch/docker), upgrades.

use tokio::task::JoinHandle;

use crate::app::App;
use crate::panel::UpgradeState;

/// Per-panel task handles.
///
/// Monitors are long-lived and reconnect. Aux views (fetch/docker) are
/// superseded on each mode switch. Upgrades outlive every view switch.
pub struct Tasks {
    pub(super) monitors: Vec<Option<JoinHandle<()>>>,
    /// View tasks -- fetch and docker. Superseded whenever the panel switches
    /// to something else, because what they produce is only worth having while
    /// it is on screen.
    pub aux: Vec<Option<JoinHandle<()>>>,
    /// Upgrade tasks, in a slot of their own.
    ///
    /// They used to share `aux`, with a parallel `aux_is_upgrade: Vec<bool>`
    /// marking the ones a view switch must not abort. The flag was obeyed and
    /// the handle was still lost: `aux[idx].replace(fetch_handle)` hands back
    /// whatever was there, and the "do not abort an upgrade" rule dropped it on
    /// the floor. The upgrade ran on with nothing tracking it -- `abort_all`
    /// could no longer reach it when the user quit, so the one thing the quit
    /// confirmation promises to stop was the one thing it could not.
    ///
    /// A separate slot makes that unrepresentable. Nothing has to remember not
    /// to abort an upgrade, because a view task and an upgrade never occupy the
    /// same place.
    pub upgrades: Vec<Option<JoinHandle<()>>>,
}

impl Tasks {
    #[must_use]
    pub fn new(n: usize) -> Self {
        Self {
            monitors: (0..n).map(|_| None).collect(),
            aux: (0..n).map(|_| None).collect(),
            upgrades: (0..n).map(|_| None).collect(),
        }
    }

    /// Start a view task on `idx`, superseding whatever view task was there.
    pub fn set_aux(&mut self, idx: usize, handle: JoinHandle<()>) {
        if let Some(old) = self.aux[idx].replace(handle) {
            old.abort();
        }
    }

    /// Start an upgrade on `idx`.
    ///
    /// Aborting whatever was in the slot is safe rather than dangerous: a
    /// second run cannot start while the first is in flight (`upgrades_in_flight`
    /// refuses it, and the resume path refuses it separately), so anything
    /// still here has already finished and `abort` on a finished task does
    /// nothing.
    pub fn set_upgrade(&mut self, idx: usize, handle: JoinHandle<()>) {
        if let Some(old) = self.upgrades[idx].replace(handle) {
            old.abort();
        }
    }

    /// Stop every upgrade this app can no longer report on.
    ///
    /// Called when the panel list is replaced: every generation moves, so no
    /// message from a task started against the old list is ever accepted again.
    /// Left running, the upgrade continues on the remote with nothing able to
    /// show it, and is then killed without a word when the app quits.
    pub fn abort_upgrades(&mut self) {
        for h in self.upgrades.iter_mut().flatten() {
            h.abort();
        }
        for slot in &mut self.upgrades {
            *slot = None;
        }
    }

    /// Grow or shrink to match a new panel count, aborting anything dropped.
    ///
    /// Without this an added server had no slot to spawn a monitor into, and a
    /// removed one left its task running against a host that is no longer shown.
    pub(super) fn fit_to(&mut self, n: usize) {
        for h in self
            .monitors
            .iter_mut()
            .skip(n)
            .chain(self.aux.iter_mut().skip(n))
            .chain(self.upgrades.iter_mut().skip(n))
            .flatten()
        {
            h.abort();
        }
        self.monitors.truncate(n);
        self.aux.truncate(n);
        self.upgrades.truncate(n);
        while self.monitors.len() < n {
            self.monitors.push(None);
            self.aux.push(None);
            self.upgrades.push(None);
        }
    }

    /// Aborting a task drops the `Child` it owns, and every child is spawned
    /// with `kill_on_drop`, so this also terminates the SSH process.
    pub(super) fn abort_all(&mut self, app: &mut App) {
        for h in self
            .monitors
            .iter_mut()
            .chain(self.aux.iter_mut())
            .chain(self.upgrades.iter_mut())
            .flatten()
        {
            h.abort();
        }
        for p in &mut app.panels {
            if p.upgrade_state == UpgradeState::STARTED {
                p.upgrade_state = UpgradeState::DONE;
            }
        }
    }
}
