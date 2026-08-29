//! Where the SSH connection-sharing socket goes, and what happens when it
//! cannot go there.
//!
//! Its own file because `support_paths_test` grew past the length cap, and this
//! is a subject rather than a leftover: the socket carries every session
//! multitop opens, including the upgrade whose stdin is the host's sudo
//! password.
//!
//! The headline is the one the old code got wrong. `ControlMaster=auto` does
//! **not** degrade when its `ControlPath` cannot be bound. Measured against
//! OpenSSH 10.3p1 with everything but the path held constant: with the
//! directory present the session multiplexes and the command runs; with it
//! missing, `ssh` prints `unix_listener: cannot bind to path ...` and the
//! command never runs at all. Not the upgrade -- every channel, on every host.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

// ----------------------------------------------------- connection sharing

/// The fallback that the old comment claimed and the code did not have.
///
/// `ControlMaster=auto` does **not** degrade when its `ControlPath` cannot be
/// bound: measured against OpenSSH 10.3p1, `ssh` prints `unix_listener: cannot
/// bind to path ...` and the command never runs. That is every channel on every
/// host, not just the upgrade, so a fresh account with no `~/.ssh` would have
/// found multitop unable to reach anything.
/// A directory short enough that the socket path still fits under the 104-byte
/// cap once `ssh` expands `%C` to forty hex characters.
///
/// `tempfile::tempdir()` is not: macOS puts it under a ~50-character
/// `/var/folders/...` prefix, which leaves no room and -- correctly -- gets no
/// sharing at all. That is the right answer to a different question, so these
/// tests ask their own.
struct ShortHome(std::path::PathBuf);

impl ShortHome {
    fn new(tag: &str) -> Self {
        let path = std::path::PathBuf::from(format!("/tmp/mt-t-{}-{tag}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("short home");
        Self(path)
    }
    fn path(&self) -> &std::path::Path {
        &self.0
    }
}

impl Drop for ShortHome {
    fn drop(&mut self) {
        use std::os::unix::fs::PermissionsExt;
        // Restore write permission first: one of these tests removes it, and a
        // directory nothing can write is a directory nothing can delete.
        if let Ok(md) = std::fs::metadata(&self.0) {
            let mut perms = md.permissions();
            perms.set_mode(0o700);
            let _ = std::fs::set_permissions(&self.0, perms);
        }
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn a_home_that_can_hold_the_socket_gets_connection_sharing() {
    let dir = ShortHome::new("share");
    let opts = multitop::ssh::multiplex_opts(dir.path()).expect("a writable home must share");
    assert!(opts.iter().any(|o| o == "ControlMaster=auto"), "{opts:?}");
    assert!(
        opts.iter().any(|o| o.starts_with("ControlPath=")),
        "{opts:?}"
    );
    // Absolute, not `~/.ssh/...`. `ssh` expands `~` from the passwd entry
    // rather than from `$HOME`, and depending on that is how an early probe of
    // this defect concluded there was no defect: it set a fake `HOME` and the
    // path it thought it was changing had not changed.
    assert!(
        opts.iter()
            .any(|o| o.starts_with(&format!("ControlPath={}", dir.path().display()))),
        "the path must be resolved by us, not by ssh: {opts:?}"
    );
    assert!(
        dir.path().join(".ssh").is_dir(),
        "the directory must be created, because ssh does not create it for a control socket"
    );
}

#[test]
fn the_socket_directory_is_owner_only() {
    use std::os::unix::fs::PermissionsExt;
    let dir = ShortHome::new("mode");
    multitop::ssh::multiplex_opts(dir.path()).expect("share");
    let mode = std::fs::metadata(dir.path().join(".ssh"))
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(
        mode, 0o700,
        "a directory anyone can write is one anyone can put a socket in first"
    );
}

/// And when it genuinely cannot be placed, the answer is no sharing rather than
/// no connection.
#[test]
fn a_home_that_cannot_hold_the_socket_gives_up_sharing_not_connecting() {
    use std::os::unix::fs::PermissionsExt;
    let dir = ShortHome::new("nowrite");
    let home = dir.path().to_path_buf();
    // Read and execute but not write, so `.ssh` cannot be created inside it.
    let mut perms = std::fs::metadata(&home).unwrap().permissions();
    perms.set_mode(0o500);
    std::fs::set_permissions(&home, perms).unwrap();

    let opts = multitop::ssh::multiplex_opts(&home);

    // Restore before the assertion, so a failure does not leave an
    // undeletable temp directory behind.
    let mut perms = std::fs::metadata(&home).unwrap().permissions();
    perms.set_mode(0o700);
    std::fs::set_permissions(&home, perms).unwrap();

    assert!(
        opts.is_none(),
        "an unusable home must drop sharing, not produce a path that fails the bind: {opts:?}"
    );
}

/// `ssh` refuses a control path at or over 104 bytes by refusing the whole
/// connection, so a home deep enough to cross that must lose sharing instead.
#[test]
fn a_path_too_long_for_a_unix_socket_drops_sharing() {
    // No filesystem: the rule is arithmetic, and the point is that a path ssh
    // would refuse is never handed to it -- including one whose directory could
    // be created perfectly well.
    let mut deep = std::path::PathBuf::from("/home/a-user");
    for _ in 0..3 {
        deep = deep.join("a-directory-with-a-fairly-long-name");
    }
    assert!(
        multitop::ssh::control_socket_path(&deep).is_none(),
        "a path ssh would refuse must not be handed to it"
    );
    assert!(multitop::ssh::multiplex_opts(&deep).is_none());
}

/// Every option that is not about sharing stays unconditional. Sharing is an
/// optimisation; `BatchMode=yes` is what stops `ssh` writing a prompt over the
/// frame, and it must never be conditional on a directory.
#[test]
fn the_options_that_protect_the_terminal_are_never_conditional() {
    assert!(
        multitop::ssh::SSH_OPTS
            .windows(2)
            .any(|p| p == ["-o", "BatchMode=yes"]),
        "{:?}",
        multitop::ssh::SSH_OPTS
    );
    assert!(
        multitop::ssh::SSH_OPTS.contains(&"-T"),
        "{:?}",
        multitop::ssh::SSH_OPTS
    );
    assert!(
        !multitop::ssh::SSH_OPTS
            .iter()
            .any(|o| o.starts_with("ControlMaster")),
        "sharing is decided at runtime, so it must not also be a constant"
    );
}
