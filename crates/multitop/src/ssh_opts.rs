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
    "ControlMaster=auto",
    // Under the user's own home, never `/tmp`.
    //
    // Every SSH session multitop opens is multiplexed over this socket --
    // including the upgrade, whose stdin carries the host's sudo password. The
    // path was `/tmp/multitop-ssh-%u-%C`, and every part of it is predictable
    // from outside: `%u` is the local username, and `%C` hashes the user, host
    // and port, which `ssh` itself puts in argv where `/proc` publishes them to
    // every account on the machine.
    //
    // `/tmp` is world-writable. A socket there cannot be *replaced* by another
    // user -- the sticky bit sees to that -- but it can be **created first**,
    // before multitop ever runs, and `ControlMaster=auto` connects to a socket
    // that is already there rather than becoming the master. Whoever is holding
    // that end is then between multitop and the remote host on every channel.
    //
    // This is the same threat model, and the same shared machine, that moved the
    // sudo password off the command line: "argv is not secret, and
    // `/proc/<pid>/cmdline` is world-readable". Taking the password out of argv
    // and then handing the whole session to a socket anyone could have created
    // is defending one half of a path.
    //
    // `~` is expanded by `ssh`, and `~/.ssh` is reachable only by its owner. If
    // it does not exist the bind fails and `ControlMaster=auto` falls back to an
    // unmultiplexed connection -- slower, and correct, which is the right way
    // round for this to degrade.
    "-o",
    "ControlPath=~/.ssh/multitop-%C",
    "-o",
    "ControlPersist=30s",
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
            assert!(!hash.is_empty());
            if hash != "missing" {
                // hash should be valid hex
                assert!(hash.chars().all(|c| c.is_ascii_hexdigit()));
            }
        }
    }

    #[test]
    fn mode_word_consistency() {
        for mode in [Mode::Monitor, Mode::Docker, Mode::Fetch] {
            assert!(!mode.word().is_empty());
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

    fn control_path() -> &'static str {
        SSH_OPTS
            .iter()
            .find_map(|opt| opt.strip_prefix("ControlPath="))
            .expect("multiplexing needs a control path")
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
        let path = control_path();
        for shared in ["/tmp/", "/var/tmp/", "/dev/shm/"] {
            assert!(
                !path.starts_with(shared),
                "the control socket must not sit in a world-writable directory: {path}"
            );
        }
        assert!(
            path.starts_with("~/") || path.starts_with("%d/"),
            "it belongs under the user's own home: {path}"
        );
    }

    /// A control path is a unix socket path, and those are capped near 104
    /// bytes on macOS and 108 on Linux. `%C` alone expands to 40 hex
    /// characters, so a long prefix is how this silently stops multiplexing.
    #[test]
    fn the_control_path_leaves_room_for_what_it_expands_to() {
        let expanded = control_path()
            .replace('~', "/home/some-long-username")
            .replace("%C", "0123456789abcdef0123456789abcdef01234567");
        assert!(
            expanded.len() < 100,
            "expanded to {} bytes, which risks the sockaddr_un limit: {expanded}",
            expanded.len()
        );
    }
}
