/// High-level application mode.
///
/// Only genuinely mutually exclusive UI states belong here. Quitting is a
/// terminal flag, orthogonal to what the UI is currently showing, so it lives
/// in its own field. Folding that kind of thing in here made opening a modal
/// silently discard an unrelated user setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AppMode {
    #[default]
    Running,
    Filtering,
    ShowUpgradeModal,
}

/// A confirmation the user is being asked, and whose keys are therefore live.
///
/// See [`App::active_confirm`]: this exists so the row on screen and the keys
/// that act cannot be chosen by two different priority orders.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Confirm {
    /// Upgrades are in flight and a quit is armed.
    Quit,
    /// The upgrade confirmation row is up.
    Upgrade,
}

/// Vault authentication state machine.
#[derive(Debug, Default)]
pub enum VaultState {
    #[default]
    Locked,
    Unlocking {
        awaiting_biometric: bool,
    },
    Unlocked {
        vault: Box<multitop_vault::UnlockedVault>,
        awaiting_biometric: bool,
    },
    PasswordPrompt {
        error: Option<String>,
    },
    /// No vault exists yet and the user is choosing a master password for a new
    /// one. Reached by saving a password with no vault present, so that the
    /// vault comes into existence the first time there is something to put in
    /// it rather than needing to be set up in advance.
    Creating {
        error: Option<String>,
        /// True once Enter has handed a master password to `Vault::initialize`
        /// and the answer has not arrived yet.
        ///
        /// Argon2id at full strength takes seconds. Without this the prompt sat
        /// there with an empty field the whole time, which reads as "it did not
        /// take" -- so the password was typed and submitted again, and again,
        /// each press initialising the same vault. The second attempt then
        /// failed (the vault now existed) and its `VaultCreateFailed` carried
        /// the same epoch as the first attempt's success, so it dropped the
        /// user back onto the creation prompt with an error after the vault was
        /// already made. That is the reported "I had to enter the vault
        /// password three times".
        in_flight: bool,
    },
}
