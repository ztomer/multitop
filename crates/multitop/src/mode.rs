//! Which view a panel shows.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Monitor,
    Docker,
    Fetch,
    Upgrade,
    /// The same Monitor stream, drawn as history rather than as a moment.
    Graphs,
    Alerts,
    /// What each host's `mcp_host` says: health verdicts, cron, container
    /// health and alerts (servers ROADMAP 12.17b). Polled while shown.
    Ops,
}

impl Mode {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Monitor => "monitor",
            Self::Docker => "docker",
            Self::Fetch => "fetch",
            Self::Upgrade => "upgrade",
            Self::Graphs => "graphs",
            Self::Alerts => "alerts",
            Self::Ops => "ops",
        }
    }

    /// Views that stay alive on the monitor stream alone, with no aux task
    /// behind them. Monitor paints from every packet; Graphs re-renders from
    /// the history those packets feed. Every other view needs something that
    /// does not survive a restart: Docker and Fetch need a one-shot aux task
    /// spawned by their toggle, Upgrade needs a run, Ops needs its poll,
    /// Alerts is rendered once
    /// at toggle time and never repainted.
    #[must_use]
    pub const fn survives_restart(self) -> bool {
        matches!(self, Self::Monitor | Self::Graphs)
    }

    /// The mode a panel actually starts in for a persisted or restored name.
    /// A task-backed view restored verbatim comes back dead: fresh panels show
    /// the initial "connecting..." body, no backing task is ever spawned for
    /// it, and the monitor stream updates history without painting. So such a
    /// view starts as Monitor instead. Applied on both sides: persist maps
    /// before writing so new state files stay clean, restore maps after
    /// parsing so state files already poisoned by older builds heal on load.
    #[must_use]
    pub const fn for_startup(self) -> Self {
        if self.survives_restart() {
            self
        } else {
            Self::Monitor
        }
    }
}

impl std::str::FromStr for Mode {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "monitor" => Ok(Self::Monitor),
            "docker" => Ok(Self::Docker),
            "fetch" => Ok(Self::Fetch),
            "upgrade" => Ok(Self::Upgrade),
            "graphs" => Ok(Self::Graphs),
            "alerts" => Ok(Self::Alerts),
            "ops" => Ok(Self::Ops),
            _ => Err(()),
        }
    }
}
