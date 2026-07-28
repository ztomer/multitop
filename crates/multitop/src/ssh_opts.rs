//! SSH connection options, Architecture types, and shell quoting helpers.

include!(concat!(env!("OUT_DIR"), "/agents.rs"));

pub const NEED_AGENT: &str = "===NEEDAGENT===";

pub const SSH_OPTS: &[&str] = &[
    "-o",
    "ControlMaster=auto",
    "-o",
    "ControlPath=/tmp/multitop-ssh-%C",
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
    pub fn from_uname(s: &str) -> Option<Arch> {
        match s.trim() {
            "x86_64" | "amd64" => Some(Arch::X86_64),
            "aarch64" | "arm64" => Some(Arch::Aarch64),
            _ => None,
        }
    }

    pub fn binary(self) -> Option<&'static [u8]> {
        match self {
            Arch::X86_64 => AGENT_X86_64,
            Arch::Aarch64 => AGENT_AARCH64,
        }
    }

    pub fn hash(self) -> &'static str {
        match self {
            Arch::X86_64 => HASH_X86_64,
            Arch::Aarch64 => HASH_AARCH64,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Arch::X86_64 => "x86_64",
            Arch::Aarch64 => "aarch64",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Monitor,
    Docker,
}

impl Mode {
    pub fn word(self) -> &'static str {
        match self {
            Mode::Monitor => "monitor",
            Mode::Docker => "docker",
        }
    }
}

pub fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}
