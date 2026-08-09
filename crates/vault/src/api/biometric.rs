//! Whether an orphaned Secure Enclave wrapper should be re-bound, and the two
//! observations that decide it.
//!
//! macOS only, and deliberately a pure rule: the enclave itself cannot run in
//! the suite, but getting the *rule* wrong is what turns a repair into an
//! enrolment, or lets a test delete the developer\'s real enclave key.

/// Decide whether a Secure Enclave wrapper is orphaned and should be re-bound
/// after a successful password unlock.
///
/// Kept as a pure function because the *rule* is the part that can be got
/// wrong, and it can be pinned by tests without a real enclave. Two of these
/// conditions are load-bearing in a way that is easy to lose:
///
/// - `has_se_wrapper` must be required, so a repair never becomes an enrolment.
///   Turning biometric unlock on for a vault that never had it is the user's
///   decision; silently adding a wrapper because the hardware happens to exist
///   would make that decision for them.
/// - `keychain_allowed` must be required, so the test suite -- which runs with
///   `use_os_keychain: false` precisely to stay off real credential storage --
///   can never generate an enclave key, and so can never delete the real one.
#[cfg(target_os = "macos")]
#[must_use]
pub(super) const fn should_rebind_biometric(
    keychain_allowed: bool,
    has_se_wrapper: bool,
    key: EnclaveKey,
    biometrics: Biometrics,
) -> bool {
    keychain_allowed
        && has_se_wrapper
        && matches!(key, EnclaveKey::Missing)
        && matches!(biometrics, Biometrics::Available)
}

/// Whether the Secure Enclave private key backing an existing wrapper still
/// loads. `Missing` covers both an absent key and one invalidated by a change
/// to the enrolled biometric set.
#[cfg(target_os = "macos")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum EnclaveKey {
    Loads,
    Missing,
}

/// Whether this machine has enrolled biometrics to bind a new key to.
#[cfg(target_os = "macos")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Biometrics {
    Available,
    Absent,
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::{should_rebind_biometric, Biometrics, EnclaveKey};

    #[test]
    fn rebinds_only_when_an_existing_wrapper_is_orphaned() {
        // The one case that should repair: a wrapper is present, its enclave key
        // is gone, and the hardware is there to make a new one.
        assert!(should_rebind_biometric(
            true,
            true,
            EnclaveKey::Missing,
            Biometrics::Available
        ));
    }

    #[test]
    fn never_rebinds_without_an_existing_wrapper() {
        // A repair must not become an enrolment. Enabling biometric unlock for a
        // vault that never had it is the user's decision.
        assert!(
            !should_rebind_biometric(true, false, EnclaveKey::Missing, Biometrics::Available),
            "no wrapper present: adding one would enable biometric unlock the user never chose"
        );
    }

    #[test]
    fn never_rebinds_when_keychain_access_is_withheld() {
        // This is what keeps the test suite from generating an enclave key --
        // and therefore from deleting the developer's real one, since
        // generation deletes first.
        assert!(
            !should_rebind_biometric(false, true, EnclaveKey::Missing, Biometrics::Available),
            "use_os_keychain=false must never reach Secure Enclave key generation"
        );
    }

    #[test]
    fn never_rebinds_while_the_existing_key_still_works() {
        assert!(
            !should_rebind_biometric(true, true, EnclaveKey::Loads, Biometrics::Available),
            "the enclave key loads fine: nothing is orphaned, so nothing to repair"
        );
    }

    #[test]
    fn never_rebinds_without_biometric_hardware() {
        assert!(
            !should_rebind_biometric(true, true, EnclaveKey::Missing, Biometrics::Absent),
            "no enrolled biometrics: generating a key would produce another dead wrapper"
        );
    }
}
