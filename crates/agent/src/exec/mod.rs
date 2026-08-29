//! Running one command on a monitored host and reporting it as framed events.
//!
//! # Why this exists
//!
//! The upgrade used to be raw text over `ssh -tt`, and the reader had to work
//! out from the bytes alone where one record ended and the next began. It could
//! not: the shape of that stream is decided outside this program. Probed
//! against one host with one command, `ssh` produced three different byte
//! streams --
//!
//! * multiplexed over a `ControlMaster` socket: a pty, lines ending `\n`;
//! * unmultiplexed (which is what a stale file at the `ControlPath` silently
//!   falls back to): a pty, lines ending `\r\n`;
//! * a local panel, which runs no `ssh` at all: no pty, two separate pipes.
//!
//! One reader, three shapes, and which one arrives depends on a file in
//! `~/.ssh`. Everything that went wrong downstream -- output logged twice, a
//! partial buffer re-emitted every 100 ms, a run that never reported finishing
//! and pinned its panel in `STARTED` for the session -- is what guessing record
//! boundaries out of that looks like in code.
//!
//! So the boundaries are put on the wire instead, in the same `MTOP` framing
//! the Monitor, Docker and Fetch channels have always used and never had this
//! class of defect in. The agent allocates the pty itself, so the shape is
//! identical whether `ssh` multiplexed, did not, or was never involved.
//!
//! # Why a pty rather than pipes
//!
//! Pipes would be less code and a worse product. Without a terminal `apt` and
//! `docker` switch to a duller output, `sudo` refuses to prompt, and an
//! interactive `Continue? [Y/n]` never reaches the panel to be answered. The
//! upgrade log exists to show what the remote tool printed; a transport that
//! changes what it prints has failed at the one job it has.

pub mod lock;
pub mod pty;
pub mod run;
pub mod script;
pub mod serve;
pub mod sieve;

/// Printed by the sudo preamble once it has turned echo off and is ready to
/// read the password.
///
/// These three live here rather than in the client because both ends now need
/// the same definition and only one of them may own it. They are recognised
/// inside the agent, on the host, by the process that owns the pty -- so they
/// are text for exactly as long as it takes the shell that printed them to hand
/// them back, and they reach the wire as [`MarkerKind`] frames or not at all.
pub const PW_READY_SENTINEL: &str = "__multitop_pw_ready__";
/// Printed by the preamble when `sudo` refuses the password it was handed.
pub const SUDO_FAILED_SENTINEL: &str = "__multitop_sudo_failed__";
/// Printed when another run holds the upgrade lock.
pub const LOCK_HELD_SENTINEL: &str = "__multitop_lock_held__";
/// Printed by the child once its login shell has finished starting and the
/// operator's command is about to run.
///
/// An interactive login shell is not quiet. `zsh -l -i` emits terminal control
/// sequences of its own before it runs anything -- two `OSC 111` on macOS --
/// and a host whose `.bashrc` prints a banner or a MOTD adds that too. All of
/// it used to land in the upgrade log above the first real line, because the
/// old transport had no way to tell the shell's noise from the tool's output.
///
/// The interactive login shell is not optional: it is what makes an alias like
/// `ud` resolve, which is what most people actually put in `upgrade_cmd`. So
/// the boundary is marked instead.
pub const STARTED_SENTINEL: &str = "__multitop_started__";
/// Printed once the operator's command has finished, before the login shell
/// unwinds.
///
/// A login shell is not quiet on the way out either: `zsh -l -i` emits an
/// `OSC 111` as it exits, observed arriving after the last line of a live `ls`
/// on a remote host. Bracketing the command between two markers is what makes
/// the log exactly the command's own output and nothing else.
pub const DONE_SENTINEL: &str = "__multitop_done__";

/// Exit status for a refused sudo password, so the outcome survives even if the
/// marker is lost. A rejected password is not a failing upgrade command: the
/// command never ran, and saying "exited 1" sends the operator to read their
/// upgrade script instead of their password.
pub const SUDO_FAILED_CODE: i32 = 111;
/// Exit status for a held lock. Also not a failing command -- it never ran.
pub const LOCK_HELD_CODE: i32 = 125;
/// Exit status when no shell could be started at all.
pub const NO_SHELL_CODE: i32 = 127;

/// The largest payload one [`ExecFrame::Out`] carries.
///
/// The `MTOP` length field is a `u16`, so a frame cannot describe itself past
/// 64 KiB. Chunking at the source is what keeps that backstop unreachable
/// rather than load-bearing: the encoder truncates an over-budget payload, and
/// a truncated `Out` would be output silently lost from an operator's log.
pub const MAX_EXEC_CHUNK: usize = 8192;

/// Which of the child's two streams a chunk came from.
///
/// Kept distinct all the way to the panel even though the pty merges them on
/// the wire, because the reason a run failed is usually on stderr and the
/// panel colours it differently.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Stream {
    Stdout = 0,
    Stderr = 1,
}

impl Stream {
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    #[must_use]
    pub const fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Stdout),
            1 => Some(Self::Stderr),
            _ => None,
        }
    }
}

/// Something the agent is telling the client, rather than showing the operator.
///
/// These were text sentinels printed into the child's own output --
/// `__multitop_pw_ready__`, `__multitop_sudo_failed__`, `__multitop_lock_held__`
/// -- which meant they were line-shaped markers in a stream whose line shape was
/// exactly the thing that varied. One of them has already been printed to an
/// operator verbatim, and the detection for another was dead on every remote
/// host for the same reason. A frame of their own cannot be mistaken for output
/// and cannot be missed for want of a newline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum MarkerKind {
    /// The child has turned echo off and is ready for the sudo password.
    PwReady = 0,
    /// `sudo` refused the password it was handed. The command never ran.
    SudoFailed = 1,
    /// Another run holds the upgrade lock. The command never ran.
    LockHeld = 2,
    /// The login shell has finished starting; what follows is the operator's
    /// command. Everything before it is the shell talking to itself.
    Started = 3,
    /// The operator's command has finished; what follows is the shell unwinding.
    Done = 4,
}

impl MarkerKind {
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    #[must_use]
    pub const fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::PwReady),
            1 => Some(Self::SudoFailed),
            2 => Some(Self::LockHeld),
            3 => Some(Self::Started),
            4 => Some(Self::Done),
            _ => None,
        }
    }
}

/// The wire tag for each frame kind.
///
/// Named, and defined once, because they were briefly written twice -- here and
/// in the decoder's match arms -- which is two definitions of one wire that can
/// disagree. `encode.rs` opens with a warning about exactly that shape: a field
/// added on one side without its counterpart on the other does not fail, it
/// shifts every field after it into the wrong slot.
pub const KIND_REQUEST: u8 = 0;
pub const KIND_BEGIN: u8 = 1;
pub const KIND_OUT: u8 = 2;
pub const KIND_MARKER: u8 = 3;
pub const KIND_ALIVE: u8 = 4;
pub const KIND_EXIT: u8 = 5;

/// One message on the exec channel, in either direction.
///
/// [`ExecFrame::Request`] travels client → agent on the agent's stdin;
/// everything else travels agent → client on its stdout. One enum and one codec
/// rather than two, because the alternative is two definitions of the same wire
/// that can disagree -- the failure mode `encode.rs` and `decode.rs` already
/// carry a warning about.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExecFrame {
    /// What to run. Carried on stdin and never in argv: `/proc/<pid>/cmdline`
    /// is world-readable, which is why the sudo password was taken out of the
    /// command line in the first place. The command joins it for the same
    /// reason -- an upgrade command names the host's package manager and often
    /// its private hosts.
    Request {
        command: String,
        password: Option<String>,
        /// Whether to take the upgrade lock. Off for a plain command, and off
        /// under the mock password store so tests do not contend on one lock.
        use_lock: bool,
        /// The window the child is told it has. A pty with a plausible size is
        /// what stops `apt` deciding it has 80 columns on a 200-column panel.
        cols: u16,
        rows: u16,
    },
    /// Sent once, before any output, so the client can report a run that
    /// started and then produced nothing.
    Begin {
        host: String,
        agent_version: String,
        pid: u32,
    },
    /// Raw bytes exactly as the child wrote them. Not a line: the agent does
    /// not know where the operator's lines are either, and pretending to would
    /// re-introduce the guess this channel exists to remove.
    Out {
        stream: Stream,
        seq: u32,
        bytes: Vec<u8>,
    },
    Marker(MarkerKind),
    /// Emitted about once a second while the child lives, so a client can tell
    /// a long compile from a wedge without waiting for a timeout that has no
    /// upper bound to be right about.
    Alive {
        elapsed_ms: u32,
    },
    /// The end of the run, and the only thing that ends it. Emitted on every
    /// path the agent can leave by.
    Exit {
        code: i32,
        signalled: bool,
    },
}

impl ExecFrame {
    /// The wire tag. Explicit rather than derived from declaration order, so
    /// reordering the enum cannot silently renumber the protocol.
    #[must_use]
    pub const fn kind(&self) -> u8 {
        match self {
            Self::Request { .. } => KIND_REQUEST,
            Self::Begin { .. } => KIND_BEGIN,
            Self::Out { .. } => KIND_OUT,
            Self::Marker(_) => KIND_MARKER,
            Self::Alive { .. } => KIND_ALIVE,
            Self::Exit { .. } => KIND_EXIT,
        }
    }

    /// Whether this frame ends the run.
    #[must_use]
    pub const fn is_terminal(&self) -> bool {
        matches!(self, Self::Exit { .. })
    }
}

/// Split `bytes` into chunks no larger than [`MAX_EXEC_CHUNK`].
///
/// Free of the `Out` frame itself so the ceiling can be tested directly: the
/// property that matters is that no chunk can make a packet the length field
/// cannot describe, and a test of that should not have to build a frame to ask.
pub fn chunks(bytes: &[u8]) -> impl Iterator<Item = &[u8]> {
    bytes.chunks(MAX_EXEC_CHUNK)
}
