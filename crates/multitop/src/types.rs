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
    /// A brand new vault was created and is unlocked, ready to receive the
    /// password whose save triggered the creation.
    /// The file now exists on disk, so the app reopens it itself rather than
    /// shipping a `Vault` through the channel (it holds no `Debug`).
    VaultCreated(Box<multitop_vault::UnlockedVault>),
    /// Creating the vault failed; the message is shown on the prompt.
    VaultCreateFailed(String),
    /// The vault was unlocked by biometric (Touch ID / fingerprint).
    VaultUnlocked(multitop_vault::UnlockedVault),
    /// Biometric unlock was unavailable or cancelled; the TUI falls back to
    /// the vault password prompt.
    VaultBiometricFailed,
}
