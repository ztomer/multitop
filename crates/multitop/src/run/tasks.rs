//! Per-panel task handles: monitors, aux views (fetch/docker), upgrades.

use std::sync::Arc;

use tokio::sync::watch;
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
    /// Ops polls, one per panel in the Ops view. A slot of their own
    /// because they are the one view task that repeats: left in `aux`, a
    /// poll outlived the view that wanted it until another view task
    /// happened to replace it. [`Tasks::retire_ops`] ends each one the
    /// moment its panel leaves the view, whatever moved it.
    pub ops: Vec<Option<JoinHandle<()>>>,
    /// The panes' render size, so a poll draws at the size of the moment
    /// rather than the size it was started at. Set by the event loop.
    pub dims: Option<Arc<watch::Receiver<(u16, u16)>>>,
}

impl Tasks {
    #[must_use]
    pub fn new(n: usize) -> Self {
        Self {
            monitors: (0..n).map(|_| None).collect(),
            aux: (0..n).map(|_| None).collect(),
            upgrades: (0..n).map(|_| None).collect(),
            ops: (0..n).map(|_| None).collect(),
            dims: None,
        }
    }

    /// Start an Ops poll on `idx`, superseding the one there.
    pub fn set_ops(&mut self, idx: usize, handle: JoinHandle<()>) {
        if let Some(old) = self.ops[idx].replace(handle) {
            old.abort();
        }
    }

    /// End the poll of every panel not in the Ops view.
    pub fn retire_ops(&mut self, panels: &[crate::panel::Panel]) {
        for (slot, p) in self.ops.iter_mut().zip(panels) {
            if p.mode != crate::panel::Mode::Ops {
                if let Some(h) = slot.take() {
                    h.abort();
                }
            }
        }
    }

    /// The render size a new poll reads: the live one, or `dims` fixed when
    /// no event loop runs (a test).
    #[must_use]
    pub fn dims_or(&self, dims: (u16, u16)) -> Arc<watch::Receiver<(u16, u16)>> {
        self.dims
            .clone()
            .unwrap_or_else(|| Arc::new(watch::channel(dims).1))
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

    /// Who is still running and who has finished silently, for a diagnostic
    /// snapshot. A finished handle with the panel still `STARTED` is the
    /// watchdog's shape of a task that died without saying so.
    pub(crate) fn diag_liveness(&self) -> crate::diag::Liveness {
        crate::diag::Liveness {
            monitors_alive: self
                .monitors
                .iter()
                .filter(|h| h.as_ref().is_some_and(|h| !h.is_finished()))
                .count(),
            upgrades_alive: self
                .upgrades
                .iter()
                .filter(|h| h.as_ref().is_some_and(|h| !h.is_finished()))
                .count(),
            upgrades_done: self
                .upgrades
                .iter()
                .filter(|h| h.as_ref().is_some_and(JoinHandle::is_finished))
                .count(),
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
            .chain(self.ops.iter_mut().skip(n))
            .flatten()
        {
            h.abort();
        }
        self.monitors.truncate(n);
        self.aux.truncate(n);
        self.upgrades.truncate(n);
        self.ops.truncate(n);
        while self.monitors.len() < n {
            self.monitors.push(None);
            self.aux.push(None);
            self.upgrades.push(None);
            self.ops.push(None);
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
            .chain(self.ops.iter_mut())
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

#[cfg(test)]
mod tests {

    use super::*;
    use crate::config::Server;
    use crate::panel::UpgradeState;

    fn server(host: &str) -> Server {
        Server {
            host: host.to_string(),
            port: 0,
            user: "a".to_string(),
            upgrade_cmd: Some("true".to_string()),
            custom_command: None,
            mcp: None,
        }
    }

    /// The three lists and the panel list are one quantity. Letting them drift
    /// means indexing a task by a panel that is not there any more.
    #[tokio::test]
    async fn the_task_lists_follow_the_panel_count_in_both_directions() {
        let mut tasks = Tasks::new(3);
        assert_eq!(tasks.monitors.len(), 3);

        tasks.fit_to(1);
        assert_eq!(tasks.monitors.len(), 1, "shrinking left stale slots behind");
        assert_eq!(tasks.aux.len(), 1);
        assert_eq!(tasks.upgrades.len(), 1);

        tasks.fit_to(4);
        assert_eq!(
            tasks.monitors.len(),
            4,
            "growing left new panels without slots"
        );
        assert_eq!(tasks.aux.len(), 4);
        assert_eq!(tasks.upgrades.len(), 4);
        assert_eq!(tasks.ops.len(), 4);
        assert!(tasks.monitors.iter().all(Option::is_none));

        // Fitting to the size it already is changes nothing.
        tasks.fit_to(4);
        assert_eq!(tasks.monitors.len(), 4);
    }

    #[tokio::test]
    async fn replacing_an_upgrade_handle_stops_the_one_it_displaces() {
        let mut tasks = Tasks::new(1);
        let first = tokio::spawn(std::future::pending::<()>());
        tasks.set_upgrade(0, first);
        tasks.set_upgrade(0, tokio::spawn(async {}));
        // The displaced task was aborted rather than left running against a
        // panel nothing reports on any more.
        tokio::task::yield_now().await;
        assert!(tasks.upgrades[0].is_some());
    }

    /// A panel left reading "running" after the loop ends never clears: it
    /// blocks every later upgrade and is recorded as an interrupted run.
    #[tokio::test]
    async fn abandoning_the_app_marks_every_running_upgrade_finished() {
        let mut app = App::new(vec![server("alpha"), server("beta")]);
        app.panels[0].upgrade_state = UpgradeState::STARTED;

        let mut tasks = Tasks::new(2);
        tasks.set_upgrade(0, tokio::spawn(std::future::pending::<()>()));
        tasks.monitors[0] = Some(tokio::spawn(std::future::pending::<()>()));
        tasks.set_aux(1, tokio::spawn(std::future::pending::<()>()));

        tasks.abort_all(&mut app);
        assert_eq!(app.panels[0].upgrade_state, UpgradeState::DONE);
        // A panel that was not running is left alone rather than being marked
        // as having finished something it never started.
        assert_ne!(app.panels[1].upgrade_state, UpgradeState::STARTED);
    }

    /// A poll runs only while its panel is in the Ops view: leaving it, by
    /// any key, ends the poll (and with it the ssh session it holds).
    #[tokio::test]
    async fn an_ops_poll_ends_when_its_panel_leaves_the_view() {
        let mut app = App::new(vec![server("alpha"), server("beta")]);
        let mut tasks = Tasks::new(2);
        app.panels[0].mode = crate::panel::Mode::Ops;
        app.panels[1].mode = crate::panel::Mode::Ops;
        tasks.set_ops(0, tokio::spawn(std::future::pending::<()>()));
        tasks.set_ops(1, tokio::spawn(std::future::pending::<()>()));
        app.panels[1].mode = crate::panel::Mode::Monitor;
        tasks.retire_ops(&app.panels);
        assert!(
            tasks.ops[0].is_some(),
            "a panel still in Ops keeps its poll"
        );
        assert!(tasks.ops[1].is_none(), "a panel that left Ops still polls");
        let fixed = tasks.dims_or((80, 24));
        assert_eq!(*fixed.borrow(), (80, 24));
    }
}
