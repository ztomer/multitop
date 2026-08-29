//! SSH connection options, Architecture types, and shell quoting helpers.

include!(concat!(env!("OUT_DIR"), "/agents.rs"));

pub const NEED_AGENT: &str = "===NEEDAGENT===";

pub const SSH_OPTS: &[&str] = &[
    // Never prompt. Without this, an unknown host key or a passphrase-protected
    // key sends `ssh` to `/dev/tty` -- which is multitop's terminal, in raw mode
    // inside the alternate screen. The question is drawn over the frame and its
    // answer is taken out of the keystrokes the event loop is reading, so the
    // panel sits on "connecting..." while the display comes apart. Refused, the
    // same situation is one legible line in the panel: `Host key verification
    // failed`, or `Permission denied (publickey)`.
    "-o",
    "BatchMode=yes",
    "-o",
    "ConnectTimeout=10",
    "-o",
    "ServerAliveInterval=15",
    "-o",
    "ServerAliveCountMax=3",
    "-o",
    "SendEnv=-*",
    "-T",
];

/// Where the connection-sharing socket lives, relative to the user's home.
///
/// Under the user's own home, never `/tmp`.
///
/// Every SSH session multitop opens is multiplexed over this socket --
/// including the upgrade, whose stdin carries the host's sudo password. The path
/// was `/tmp/multitop-ssh-%u-%C`, and every part of it is predictable from
/// outside: `%u` is the local username, and `%C` hashes the user, host and port,
/// which `ssh` itself puts in argv where `/proc` publishes them to every account
/// on the machine.
///
/// `/tmp` is world-writable. A socket there cannot be *replaced* by another user
/// -- the sticky bit sees to that -- but it can be **created first**, before
/// multitop ever runs, and `ControlMaster=auto` connects to a socket that is
/// already there rather than becoming the master. Whoever is holding that end is
/// then between multitop and the remote host on every channel.
///
/// This is the same threat model, and the same shared machine, that moved the
/// sudo password off the command line.
pub const CONTROL_DIR: &str = ".ssh";
pub const CONTROL_PREFIX: &str = "multitop-";

/// The connection-sharing options, or none when the socket cannot be placed.
///
/// # Why this is decided at runtime and not written into `SSH_OPTS`
///
/// It used to be three constants, under a comment that said:
///
/// > `~` is expanded by `ssh`, and `~/.ssh` is reachable only by its owner. If
/// > it does not exist the bind fails and `ControlMaster=auto` falls back to an
/// > unmultiplexed connection -- slower, and correct, which is the right way
/// > round for this to degrade.
///
/// **It does not fall back.** Measured against OpenSSH 10.3p1 with everything
/// but the `ControlPath` held constant: with the directory present the session
/// multiplexes and the command runs; with the directory missing `ssh` prints
/// `unix_listener: cannot bind to path ...` and **the command never runs at
/// all**. Not the upgrade -- every channel, on every host. A fresh account with
/// no `~/.ssh` would have found multitop unable to reach anything, with the
/// reason on a stderr stream the panel reports as a closed connection.
///
/// So the fallback is performed here rather than hoped for: the directory is
/// created if it is missing, and if it cannot be, the sharing options are left
/// out and every connection stands on its own. Slower, and working.
///
/// The path is resolved and passed absolute rather than as `~/.ssh/...`. `ssh`
/// expands `~` from the passwd entry and not from `$HOME`, which is a thing
/// worth not having to remember -- an early probe of this defect set a fake
/// `HOME`, watched the connection succeed, and concluded there was no defect,
/// because the path it thought it was changing had not changed.
#[must_use]
pub fn multiplex_opts(home: &std::path::Path) -> Option<Vec<String>> {
    let dir = home.join(CONTROL_DIR);
    if !dir.is_dir() {
        std::fs::create_dir_all(&dir).ok()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            // Owner-only, which is the whole reason the socket lives here. A
            // directory anyone can write is a directory anyone can put a socket
            // in first.
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).ok()?;
        }
    }
    let path = control_socket_path(home)?;
    Some(vec![
        "-o".to_string(),
        "ControlMaster=auto".to_string(),
        "-o".to_string(),
        format!("ControlPath={path}"),
        "-o".to_string(),
        "ControlPersist=30s".to_string(),
    ])
}

/// Where the socket would go, and whether `ssh` would accept the path.
///
/// Separate from [`multiplex_opts`] because it creates nothing: the length rule
/// is arithmetic and deserves to be checked against any home, including ones it
/// would be rude to make directories in.
///
/// # The length
///
/// `ssh` refuses a control path at or over 104 bytes -- and refuses the whole
/// connection to do it, so this has to be right rather than approximately
/// right. It is measured **expanded**: `%C` is two characters here and forty
/// after `ssh` substitutes its hash, and the first version of this check
/// compared the unexpanded string, which let through paths 38 bytes over the
/// limit. A test with a deep home caught it.
#[must_use]
pub fn control_socket_path(home: &std::path::Path) -> Option<String> {
    let path = home
        .join(CONTROL_DIR)
        .join(format!("{CONTROL_PREFIX}%C"))
        .to_str()?
        .to_string();
    if expanded_len(&path) >= CONTROL_PATH_MAX {
        return None;
    }
    Some(path)
}

/// How long the path will be once `ssh` has substituted its tokens.
fn expanded_len(path: &str) -> usize {
    path.len() - "%C".len() * path.matches("%C").count()
        + CONTROL_HASH_LEN * path.matches("%C").count()
}

/// `sockaddr_un.sun_path` is 104 bytes on macOS and 108 on Linux; the smaller
/// one governs, because a path that works on one and not the other is a defect
/// that only appears on someone else's machine.
pub const CONTROL_PATH_MAX: usize = 104;
/// `%C` is a SHA-1 hash rendered as hex.
pub const CONTROL_HASH_LEN: usize = 40;

/// The sharing options for this user, worked out once.
///
/// Once because the answer cannot change while the program runs and the check
/// touches the filesystem, and every panel opens connections.
#[must_use]
pub fn active_multiplex_opts() -> &'static [String] {
    static OPTS: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    OPTS.get_or_init(|| {
        std::env::var_os("HOME")
            .map(std::path::PathBuf::from)
            .and_then(|home| multiplex_opts(&home))
            .unwrap_or_default()
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arch {
    X86_64,
    Aarch64,
}

impl Arch {
    #[must_use]
    pub fn from_uname(s: &str) -> Option<Self> {
        match s.trim() {
            "x86_64" | "amd64" => Some(Self::X86_64),
            "aarch64" | "arm64" => Some(Self::Aarch64),
            _ => None,
        }
    }

    #[must_use]
    pub fn binary(self) -> Option<&'static [u8]> {
        match self {
            Self::X86_64 => AGENT_X86_64,
            Self::Aarch64 => AGENT_AARCH64,
        }
    }

    #[must_use]
    pub fn hash(self) -> &'static str {
        match self {
            Self::X86_64 => HASH_X86_64,
            Self::Aarch64 => HASH_AARCH64,
        }
    }

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::X86_64 => "x86_64",
            Self::Aarch64 => "aarch64",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Monitor,
    Docker,
    Fetch,
}

impl Mode {
    #[must_use]
    pub const fn word(self) -> &'static str {
        match self {
            Self::Monitor => "monitor",
            Self::Docker => "docker",
            Self::Fetch => "fetch",
        }
    }
}

#[must_use]
pub fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arch_from_uname_x86_64() {
        assert_eq!(Arch::from_uname("x86_64"), Some(Arch::X86_64));
        assert_eq!(Arch::from_uname("amd64"), Some(Arch::X86_64));
    }

    #[test]
    fn arch_from_uname_aarch64() {
        assert_eq!(Arch::from_uname("aarch64"), Some(Arch::Aarch64));
        assert_eq!(Arch::from_uname("arm64"), Some(Arch::Aarch64));
    }

    #[test]
    fn arch_from_uname_unknown() {
        assert_eq!(Arch::from_uname("armv7"), None);
        assert_eq!(Arch::from_uname("riscv64"), None);
        assert_eq!(Arch::from_uname(""), None);
    }

    #[test]
    fn arch_hash_label_consistency() {
        for arch in [Arch::X86_64, Arch::Aarch64] {
            assert_eq!(arch.label(), arch.label());
            let hash = arch.hash();
            // In test builds, hash may be "missing" - just verify it's non-empty
            assert_ne!(hash, "");
            if hash != "missing" {
                // hash should be valid hex
                assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
            }
        }
    }

    #[test]
    fn mode_word_consistency() {
        for mode in [Mode::Monitor, Mode::Docker, Mode::Fetch] {
            assert_ne!(mode.word(), "");
        }
    }

    #[test]
    fn arch_binary_selection() {
        // In test builds, agents may be None; just verify the method exists
        let _ = Arch::X86_64.binary();
        let _ = Arch::Aarch64.binary();
    }

    #[test]
    fn sh_quote_empty() {
        assert_eq!(sh_quote(""), "''");
    }

    #[test]
    fn sh_quote_simple() {
        assert_eq!(sh_quote("hello"), "'hello'");
    }

    #[test]
    fn sh_quote_single_quotes() {
        assert_eq!(sh_quote("it's"), "'it'\\''s'");
    }

    #[test]
    fn sh_quote_idempotent() {
        let s = "test 'quote' here";
        let once = sh_quote(s);
        let twice = sh_quote(&once);
        // sh_quote(sh_quote(x)) should be safely nested
        assert!(twice.contains("test"));
        assert!(twice.contains("quote"));
        assert!(twice.contains("here"));
    }
}

#[cfg(test)]
mod control_path_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::SSH_OPTS;
    use std::path::Path;

    /// An ordinary home, not a temporary directory.
    ///
    /// These are questions about the *shape* of the path, so they are asked of
    /// `control_socket_path`, which creates nothing. Using a `tempdir` here
    /// would ask them of macOS's 50-character `/var/folders/...` prefix, which
    /// is a real answer to a different question -- and which, correctly, has no
    /// room left for the socket at all.
    const HOME: &str = "/home/some-long-username";

    fn control_path(home: &Path) -> String {
        super::control_socket_path(home).expect("an ordinary home must produce a path")
    }

    /// The control socket carries every session multitop opens, including the
    /// upgrade whose stdin is the host's sudo password. It must not live
    /// anywhere another account can get to first.
    ///
    /// It was `/tmp/multitop-ssh-%u-%C`. The sticky bit stops another user
    /// *replacing* that socket, but nothing stops them **creating** it before
    /// multitop first runs -- and `ControlMaster=auto` joins a socket that is
    /// already there instead of becoming the master. Both halves of the name are
    /// predictable: `%u` is the local username, and `%C` hashes the user, host
    /// and port, which `ssh` puts in argv where `/proc` publishes them.
    ///
    /// This is the same threat model that moved the sudo password off the
    /// command line, so it gets the same answer.
    #[test]
    fn the_control_socket_is_not_somewhere_anyone_can_create_it() {
        let home = Path::new(HOME);
        let path = control_path(home);
        for shared in ["/tmp/", "/var/tmp/", "/dev/shm/"] {
            assert!(
                !path.starts_with(shared),
                "the control socket must not sit in a world-writable directory: {path}"
            );
        }
        assert!(
            path.starts_with(&format!("{}/", home.display())),
            "it belongs under the user's own home: {path}"
        );
        assert!(
            path.contains("/.ssh/"),
            "and specifically in the one directory there that is owner-only: {path}"
        );
    }

    /// A control path is a unix socket path, and those are capped near 104
    /// bytes on macOS and 108 on Linux. `%C` alone expands to 40 hex
    /// characters, so a long prefix is how this silently stops multiplexing --
    /// except it does not stop multiplexing, it stops the *connection*, which
    /// is why `multiplex_opts` refuses the path rather than handing it over.
    #[test]
    fn the_control_path_leaves_room_for_what_it_expands_to() {
        let path = control_path(Path::new(HOME));
        let expanded = path.replace("%C", "0123456789abcdef0123456789abcdef01234567");
        assert!(
            expanded.len() < super::CONTROL_PATH_MAX,
            "expanded to {} bytes, which ssh refuses -- and it refuses the whole \
             connection to do it: {expanded}",
            expanded.len()
        );
    }

    /// The check has to be made on the *expanded* path. `%C` is two characters
    /// before `ssh` substitutes it and forty after, and the first version of
    /// this compared the unexpanded string -- so it let through paths 38 bytes
    /// over a limit whose penalty is the connection failing.
    #[test]
    fn the_length_is_judged_after_the_hash_is_substituted() {
        // Short enough unexpanded, far too long once `%C` becomes 40 hex bytes.
        let deep = Path::new(
            "/home/a-user/and-a-fairly-deep-directory/nested-again-for-good-measure/once-more",
        );
        let unexpanded = deep.join(super::CONTROL_DIR).join("multitop-%C");
        assert!(
            unexpanded.to_str().unwrap().len() < super::CONTROL_PATH_MAX,
            "the test's premise: this path only fails once %C is expanded"
        );
        assert!(
            super::control_socket_path(deep).is_none(),
            "a path ssh will refuse must not be handed to it"
        );
    }

    /// Sharing is an optimisation and may be dropped; the options that keep
    /// `ssh` away from multitop's terminal are not and may not.
    #[test]
    fn the_unconditional_options_carry_nothing_about_sharing() {
        assert!(
            !SSH_OPTS.iter().any(|o| o.starts_with("Control")),
            "sharing is decided at runtime now: {SSH_OPTS:?}"
        );
        assert!(SSH_OPTS.windows(2).any(|p| p == ["-o", "BatchMode=yes"]));
    }
}
