use multitop_agent::fetch::FetchSnapshot;

/// Work the runtime should start as a result of a state transition.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    RunDocker { panel: usize, gen: u64 },
    RunFetch { panel: usize, gen: u64 },
    RunUpgrade { panel: usize, gen: u64 },
}

/// Messages produced by the background tasks.
#[derive(Debug)]
pub enum Msg {
    Packet {
        panel: usize,
        gen: u64,
        payload: multitop_agent::proto::Payload,
        dims: (u16, u16),
    },
    Frame {
        panel: usize,
        lines: Vec<String>,
    },
    Status {
        panel: usize,
        gen: u64,
        text: String,
    },
    FetchData {
        panel: usize,
        gen: u64,
        snap: FetchSnapshot,
        lines: Vec<String>,
    },
    AuxBegin {
        panel: usize,
        gen: u64,
        header: Option<String>,
    },
    AuxLine {
        panel: usize,
        gen: u64,
        line: String,
    },
    AuxDone {
        panel: usize,
        gen: u64,
        note: Option<String>,
        success: bool,
    },
    /// The vault was unlocked by biometric (Touch ID / fingerprint).
    VaultUnlocked(multitop_vault::UnlockedVault),
    /// Biometric unlock was unavailable or cancelled; the TUI falls back to
    /// the vault password prompt.
    VaultBiometricFailed,
}
