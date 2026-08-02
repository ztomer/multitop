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
        /// Which panel *list* the sender was started for.
        ///
        /// Deliberately not the per-panel `gen`, which counts operations and is
        /// bumped by every mode switch: a monitor task is spawned once and lives
        /// across those, so matching on `gen` would reject its frames the first
        /// time the user pressed `d` and freeze the stats panel for good. This
        /// changes only when the panel list itself is replaced, which is exactly
        /// when a captured index stops meaning the same host.
        epoch: u64,
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
    ///
    /// The `epoch` is the vault-operation token in force when the attempt
    /// started. A result whose epoch is stale belongs to an attempt the user
    /// has since cancelled and is discarded.
    VaultCreated {
        epoch: u64,
        unlocked: Box<multitop_vault::UnlockedVault>,
    },
    /// Creating the vault failed; the message is shown on the prompt.
    VaultCreateFailed { epoch: u64, error: String },
    /// A password unlock attempt finished unsuccessfully. Carries the reason so
    /// the prompt can show it.
    VaultUnlockFailed { epoch: u64, error: String },
    /// The vault was unlocked by biometric (Touch ID / fingerprint).
    VaultUnlocked {
        epoch: u64,
        unlocked: Box<multitop_vault::UnlockedVault>,
    },
    /// Biometric unlock was unavailable or cancelled; the TUI falls back to
    /// the vault password prompt.
    VaultBiometricFailed { epoch: u64 },
}
