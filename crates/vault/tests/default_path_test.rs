//! The default vault path, pinned against the platform convention rather than
//! against the `dirs` crate that computes it.
//!
//! `VaultConfig::default` asks `dirs::data_local_dir` where the vault lives. A
//! `dirs` major that moved that directory would not fail loudly: the vault would
//! open as a fresh empty one where the real one was, with no error. A test that
//! compared the default against `dirs` would agree with whatever `dirs` said, so
//! the expected path here is built from `$HOME` (and `$XDG_DATA_HOME` on Linux)
//! the way the platform defines it -- an instrument independent of the thing it
//! measures. It went in with the `dirs` 5 -> 7 move, which left the path where
//! it was; the next move that does not is red here.

// A test crate, said where clippy reads it (see atomic_write_test.rs).
#![cfg(test)]

use std::path::PathBuf;

fn home() -> PathBuf {
    PathBuf::from(std::env::var_os("HOME").expect("HOME is set for the test run"))
}

/// `~/Library/Application Support`, Apple's per-user application data root.
#[cfg(target_os = "macos")]
fn data_local_dir() -> PathBuf {
    home().join("Library").join("Application Support")
}

/// The XDG base-directory rule: `$XDG_DATA_HOME` when it is absolute, else
/// `~/.local/share`.
#[cfg(target_os = "linux")]
fn data_local_dir() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home().join(".local").join("share"))
}

#[test]
fn default_vault_path_follows_the_platform_convention() {
    let expected = data_local_dir().join("multitop").join("vault.bin");
    assert_eq!(
        multitop_vault::VaultConfig::default().vault_path,
        expected,
        "the default vault path moved -- an existing vault would open as a new empty one"
    );
}
