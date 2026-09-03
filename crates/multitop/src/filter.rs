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
    /// the cache is what was drawn. `unhealthy` is a synthetic token that
    /// checks `health` rather than text, and a `/.../` query is a regex.
    #[must_use]
    pub fn matches_filter(&self, query: &str) -> bool {
        let q = query.trim().to_lowercase();
        if q.is_empty() {
            return true;
        }
        // `unhealthy` — matches hosts breaching any alert threshold.
        if q == "unhealthy" {
            if let Some(multitop_agent::proto::Payload::Monitor(snap)) = &self.last_monitor {
                // Use a dummy config with default thresholds (80/85/90) when the
                // real config is not at hand; the panel doesn't carry it.
                // A host that is actually breaching will still match, and a
                // healthy host will not be invented as breaching.
                let cfg = crate::config::Config {
                    servers: vec![],
                    theme: None,
                    upgrade_history_lines: 5000,
                    history_lines_raised_from: None,
                    banner_style: crate::layout::BannerStyle::default(),
                    plaintext_passwords: vec![],
                    alert_cpu: None,
                    alert_mem: None,
                    alert_disk: None,
                    alerts: vec![],
                };
                return crate::health::is_breaching(snap, &cfg);
            }
            return false;
        }
        // `ip:1.2.3.4` — match host IP.
        if let Some(ip) = q.strip_prefix("ip:") {
            let ip = ip.trim();
            if !ip.is_empty() && self.server.host.contains(ip) {
                return true;
            }
        }
        // `image:nginx` — match docker image.
        if let Some(img) = q.strip_prefix("image:") {
            let img = img.trim();
            if !img.is_empty() {
                if let Some(multitop_agent::proto::Payload::Docker { rows, .. }) = &self.last_docker
                {
                    if rows.iter().any(|r| r.image.to_lowercase().contains(img)) {
                        return true;
                    }
                }
                // Also check fetch? No, image only in docker.
            }
        }
        // `cpu>50`, `cpu>=50`, `cpu<50`, `mem>80` etc.
        for prefix in [
            "cpu>=", "cpu<=", "cpu>", "cpu<", "mem>=", "mem<=", "mem>", "mem<",
        ] {
            if let Some(num_str) = q.strip_prefix(prefix) {
                if let Ok(threshold) = num_str.trim().parse::<f64>() {
                    if let Some(multitop_agent::proto::Payload::Monitor(snap)) = &self.last_monitor
                    {
                        let value = if prefix.starts_with("cpu") {
                            snap.cpu_pct
                        } else {
                            snap.mem.pct
                        };
                        let cmp = if prefix.contains(">=") {
                            value >= threshold
                        } else if prefix.contains("<=") {
                            value <= threshold
                        } else if prefix.contains('>') {
                            value > threshold
                        } else {
                            value < threshold
                        };
                        if cmp {
                            return true;
                        }
                    }
                }
            }
        }
        // `/.../` regex — `(?i)` already via lowercasing, so just try.
        if q.starts_with('/') && q.ends_with('/') && q.len() > 2 {
            let pat = &q[1..q.len() - 1];
            if let Ok(re) = regex::Regex::new(pat) {
                let haystack = |text: &str| re.is_match(text);
                if haystack(&self.server.host) || haystack(&self.server.user) {
                    return true;
                }
                return match self.mode {
                    Mode::Monitor | Mode::Graphs | Mode::Alerts => self.monitor_matches(&haystack),
                    Mode::Docker => self.docker_matches(&haystack),
                    Mode::Fetch => self.fetch_matches(&haystack),
                    Mode::Upgrade => self.last_upgrade.iter().any(|l| haystack(l)),
                };
            }
        }
        let hit = |text: &str| text.to_lowercase().contains(&q);

        if hit(&self.server.host) || hit(&self.server.user) {
            return true;
        }
        match self.mode {
            // Graphs draws the same Monitor stream, so it searches the same
            // thing. Anything else would make `G` change what `/` finds.
            Mode::Monitor | Mode::Graphs | Mode::Alerts => self.monitor_matches(&hit),
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
