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
        /// The panel list this packet was produced for.
        ///
        /// `gen` alone cannot guard the monitor stream: those tasks are
        /// long-lived and stamp every packet `gen: 0`, so once a server edit
        /// moves every generation off zero, a `gen` check would reject live
        /// stats forever -- which is why the arm that handles them checked
        /// nothing at all, and one machine's statistics could be painted under
        /// another machine's name after a deletion. That is the exact defect
        /// `replace_panels` bumps the epoch to prevent, reachable through the
        /// one arm that never consulted it. Carrying the epoch is what lets
        /// every arm be guarded by something.
        epoch: u64,
        payload: multitop_agent::proto::Payload,
        dims: (u16, u16),
    },
    /// The master password was replaced.
    VaultPasswordRotated { epoch: u64 },
    /// The replacement did not happen; the old password still works.
    VaultPasswordRotationFailed { epoch: u64, error: String },
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
    /// A line that replaces one already in the log, rather than following it.
    ///
    /// A tool that repaints a block — `docker compose pull` — moves the cursor
    /// up over it with `ESC[nA` and writes the block again. Treated as new
    /// lines, every tick added another copy of the block and the run drowned in
    /// its own progress display. A separate message rather than a flag on
    /// `AuxLine`, because "this replaces what you are showing" and "here is
    /// something new" are different events, and the reader is the only place
    /// that still sees the control sequences that tell them apart.
    AuxRepaint {
        panel: usize,
        gen: u64,
        line: String,
        /// Lines back from the newest, where 0 is the newest.
        back: usize,
        /// Rows below that one which the tool just erased.
        erase_below: usize,
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
