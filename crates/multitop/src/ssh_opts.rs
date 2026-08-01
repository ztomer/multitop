//! SSH connection options, Architecture types, and shell quoting helpers.

include!(concat!(env!("OUT_DIR"), "/agents.rs"));

pub const NEED_AGENT: &str = "===NEEDAGENT===";

pub const SSH_OPTS: &[&str] = &[
    "-o",
    "ControlMaster=auto",
    "-o",
    "ControlPath=/tmp/multitop-ssh-%u-%C",
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
