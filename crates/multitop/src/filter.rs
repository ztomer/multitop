//! Deciding whether a panel matches what the user is typing into `/`.
//!
//! Its own module because the question is about *searching*, not about panel
//! state, and because the answer depends on which view the panel is in -- so it
//! is a small dispatch table that would otherwise sit in the middle of
//! `panel.rs` having nothing to do with its neighbours.
//!
//! The rule, in one line: a panel matches what it is currently showing. Typing
//! a process name while looking at the stats view should find the hosts running
//! it; the same query in the Docker view has no business searching a process
//! table nobody is looking at, and would silently give a different answer
//! depending on where the user had been.

use crate::panel::{Mode, Panel};

impl Panel {
    /// Does this panel match a filter query?
    ///
    /// Against what the panel is *currently showing*, not a fixed pair of
    /// fields. Typing a process name while looking at the stats view should
    /// find the hosts running it; the same query in the Docker view has no
    /// business searching a process table nobody is looking at, and would
    /// silently give a different answer depending on where the user had been.
    ///
    /// The host and user always count, in every view: the banner draws the host
    /// name on every pane whatever is underneath it, so a name on screen that
    /// the filter cannot find would be the filter lying about what it searched.
    ///
    /// Only cached payloads are consulted, which is exactly the same rule --
    /// the cache is what was drawn.
    #[must_use]
    pub fn matches_filter(&self, query: &str) -> bool {
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return true;
        }
        let hit = |text: &str| text.to_lowercase().contains(&q);

        if hit(&self.server.host) || hit(&self.server.user) {
            return true;
        }
        match self.mode {
            // Graphs draws the same Monitor stream, so it searches the same
            // thing. Anything else would make `G` change what `/` finds.
            Mode::Monitor | Mode::Graphs => self.monitor_matches(&hit),
            Mode::Docker => self.docker_matches(&hit),
            Mode::Fetch => self.fetch_matches(&hit),
            Mode::Upgrade => self.last_upgrade.iter().any(|l| hit(l)),
        }
    }

    /// The host as the agent reports it, and everything it is running.
    ///
    /// `proc_names`, not just `procs`. The table is capped to what fits the
    /// pane, so matching against it meant `/` could only find a host by a
    /// process near the top of its table -- and because filtering makes panes
    /// bigger, the cap rose and the same query answered differently a second
    /// later. `procs` is still searched, for a packet from an agent too old to
    /// send the full list.
    fn monitor_matches(&self, hit: &impl Fn(&str) -> bool) -> bool {
        let Some(multitop_agent::proto::Payload::Monitor(snap)) = &self.last_monitor else {
            return false;
        };
        hit(&snap.host)
            || snap.proc_names.iter().any(|n| hit(n))
            || snap.procs.iter().any(|p| hit(&p.name))
    }

    /// Container names, images and status text.
    fn docker_matches(&self, hit: &impl Fn(&str) -> bool) -> bool {
        let Some(multitop_agent::proto::Payload::Docker { host, rows }) = &self.last_docker else {
            return false;
        };
        hit(host)
            || rows
                .iter()
                .any(|r| hit(&r.name) || hit(&r.image) || hit(&r.status))
    }

    /// Everything the Fetch card prints.
    fn fetch_matches(&self, hit: &impl Fn(&str) -> bool) -> bool {
        let Some(s) = &self.last_fetch else {
            return false;
        };
        hit(&s.user_host) || hit(&s.os) || hit(&s.kernel) || hit(&s.host_model) || hit(&s.cpu_model)
    }
}
