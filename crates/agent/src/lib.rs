//! Compact system and Docker monitor.
//!
//! Built as a small static binary that `multitop` uploads to each monitored
//! host and runs over SSH. It writes frames to stdout; when stdout is a
//! terminal it repaints in place, otherwise it delimits frames with
//! `===MONITOR===` so the reader can tell them apart.
//!
//! # `unsafe_code`
//!
//! The workspace denies it. This crate is the one that cannot honour that: its
//! entire job is `/proc`, `sysctl`, `openpty`, `flock`. Rather than lift the
//! deny crate-wide, each module that reaches for libc names itself with a
//! module-level attribute, so a NEW module still cannot add unsafe silently.
//! The FFI modules are `exec/lock`, `exec/pty`, `exec/run`, `proc`, `proc_sys`,
//! `fetch`, `sys` and `sys_temps`. The last three use `allow` rather than
//! `expect` because their unsafe is `cfg(target_os = "macos")`: on Linux it
//! disappears, and an unfulfilled `expect` under `-D warnings` is an error.

use std::os::unix::fs::FileTypeExt;

pub mod color;
pub mod consts;
pub mod cpufreq;
pub mod docker;
pub mod docker_cli;
pub mod docker_render;
pub mod docker_transport;
pub mod emit;
pub mod exec;
pub mod fetch;
pub mod fmt;
pub mod monitor;
pub mod proc;
pub mod proc_disk;
pub mod proc_sys;
pub mod proto;
pub mod render;
pub mod render_layout;
pub mod sys;
pub mod sys_temps;

// Re-exported so the split is invisible to callers.
use emit::emit_hello;
pub use emit::{emit_docker, emit_fetch, emit_monitor};

/// Frame delimiter on the wire between agent and TUI.
pub const FRAME_MARKER: &str = "===MONITOR===";

/// Default refresh interval, in seconds.
pub const DEFAULT_INTERVAL: f64 = 2.0;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SortBy {
    #[default]
    Cpu,
    Mem,
}

impl SortBy {
    #[must_use]
    pub const fn word(&self) -> &'static str {
        match self {
            Self::Cpu => "cpu",
            Self::Mem => "mem",
        }
    }
}

/// Parsed command line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Args {
    pub mode: Mode,
    /// What the user asked for that is not a mode: usage, or the version.
    ///
    /// Its own field rather than a mode, because these do not sample anything
    /// and must not reach the render loop.
    pub tell: Option<Tell>,
    pub display_ip: Option<String>,
    pub cols: usize,
    pub lines: usize,
    pub sort: SortBy,
}

/// Something to print and exit, rather than a thing to run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tell {
    Usage,
    Version,
    /// A flag this build does not know. Reported rather than ignored: the first
    /// positional argument is the host label, so an unrecognised `--flag` used
    /// to become the *name of the host* -- `multitop-agent --help` printed a
    /// binary monitor packet naming a host called `beelink (--help)`, on a
    /// binary that is uploaded to every machine being monitored.
    Unknown(&'static str),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Monitor,
    Docker,
    Fetch,
    /// Run one command and report it as frames. What it runs arrives on stdin,
    /// never in argv: `/proc/<pid>/cmdline` is world-readable, and the request
    /// carries a sudo password.
    Exec,
}

impl Mode {
    #[must_use]
    pub const fn word(&self) -> &'static str {
        match self {
            Self::Monitor => "monitor",
            Self::Docker => "docker",
            Self::Fetch => "fetch",
            Self::Exec => "exec",
        }
    }
}

impl Default for Args {
    fn default() -> Self {
        Self {
            mode: Mode::Monitor,
            tell: None,
            display_ip: None,
            cols: 80,
            lines: 24,
            sort: SortBy::Cpu,
        }
    }
}

/// Parse `[monitor|docker|fetch] [ip] [cols] [lines] [cpu|mem]`.
///
/// The mode word is optional so the binary still behaves sensibly when run by
/// hand on a server with no arguments at all.
pub fn parse_args<I: IntoIterator<Item = String>>(raw: I) -> Args {
    let mut args = Args::default();
    let mut rest: Vec<String> = raw.into_iter().collect();

    if let Some(first) = rest.first() {
        match first.as_str() {
            "monitor" => {
                args.mode = Mode::Monitor;
                rest.remove(0);
            }
            "docker" => {
                args.mode = Mode::Docker;
                rest.remove(0);
            }
            "fetch" => {
                args.mode = Mode::Fetch;
                rest.remove(0);
            }
            "exec" => {
                args.mode = Mode::Exec;
                rest.remove(0);
            }
            _ => {}
        }
    }

    // Flags before positionals, because a positional is a host label and would
    // otherwise swallow them.
    if let Some(first) = rest.first() {
        args.tell = match first.as_str() {
            "--help" | "-h" | "help" => Some(Tell::Usage),
            "--version" | "-V" => Some(Tell::Version),
            f if f.starts_with('-') => Some(Tell::Unknown("unrecognised option")),
            _ => None,
        };
        if args.tell.is_some() {
            return args;
        }
    }

    let mut positional = rest.into_iter();
    args.display_ip = positional.next().filter(|s| !s.is_empty());
    if let Some(v) = positional.next().and_then(|v| v.parse().ok()) {
        args.cols = v;
    }
    if let Some(v) = positional.next().and_then(|v| v.parse().ok()) {
        args.lines = v;
    }
    if let Some(v) = positional.next() {
        match v.to_ascii_lowercase().as_str() {
            "mem" | "memory" => args.sort = SortBy::Mem,
            "cpu" => args.sort = SortBy::Cpu,
            _ => {}
        }
    }
    args
}

/// Columns sampled for the binary stream. The client re-renders locally at
/// whatever size its panel happens to be, so the agent samples a generous
/// fixed budget rather than guessing.
const STREAM_COLS: usize = 120;
/// Lines sampled for the binary stream — the process-list budget follows from
/// this, and 50 is more rows than any panel draws.
const STREAM_LINES: usize = 50;

/// How much to sample for one frame: exactly what this terminal draws, or the
/// stream budget when the far end does the drawing.
const fn sample_dims(args: &Args, is_tty: bool) -> (usize, usize) {
    if is_tty {
        (args.cols, args.lines)
    } else {
        (STREAM_COLS, STREAM_LINES)
    }
}

/// Which palette the environment asks for.
#[must_use]
pub fn palette_for_env() -> &'static color::Palette {
    if std::env::var_os("NO_COLOR").is_some() {
        &color::PLAIN
    } else {
        &color::ANSI
    }
}

/// Repaint until the reader goes away.
///
/// `next_tick` returns the seconds since the previous frame, or `None` when
/// the loop should stop — which is where the "stdin closed" signal and, in a
/// test, a fixed frame count both enter. The first frame is emitted before
/// the first wait so a client sees data the moment it connects.
pub fn monitor_loop<W: std::io::Write>(
    monitor: &mut monitor::Monitor,
    args: &Args,
    is_tty: bool,
    pal: &color::Palette,
    out: &mut W,
    next_tick: &mut dyn FnMut() -> Option<f64>,
) {
    if !is_tty {
        let _ = emit_hello(out);
        let _ = out.flush();
    }
    let (cols, lines) = sample_dims(args, is_tty);
    let mut buf = String::with_capacity(consts::FRAME_BUF_CAPACITY);
    let mut elapsed = 0.0;
    loop {
        let snap = monitor.tick(elapsed, cols, lines, args.sort);
        if emit_monitor(&snap, args, is_tty, pal, &mut buf, out).is_err() {
            return;
        }
        match next_tick() {
            Some(secs) => elapsed = secs,
            None => return,
        }
    }
}

/// Watch stdin for EOF, which is how the agent learns its reader is gone.
///
/// Only a pipe is watched: a terminal's stdin belongs to the user, and a
/// closed one there means nothing.
#[must_use]
pub fn stdin_eof_watcher() -> std::sync::Arc<std::sync::atomic::AtomicBool> {
    use std::io::{self, IsTerminal, Read};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    let stdin_gone = Arc::new(AtomicBool::new(false));
    let stdin_pipe = !io::stdin().is_terminal()
        && std::fs::metadata("/proc/self/fd/0").is_ok_and(|m| m.file_type().is_fifo());
    if stdin_pipe {
        let sig = stdin_gone.clone();
        std::thread::spawn(move || {
            let mut buf = [0u8; consts::STDIN_WATCH_BUF];
            while let Ok(n) = io::stdin().read(&mut buf) {
                if n == 0 {
                    break;
                }
            }
            sig.store(true, Ordering::Relaxed);
        });
    }
    stdin_gone
}

/// What the agent prints when asked what it is.
///
/// Plain text on stdout. This binary is uploaded to every monitored host, so
/// the person most likely to run it by hand is an operator who found it in
/// `/usr/local/bin` and wants to know what it is -- and what they used to get
/// was a monitor packet.
#[must_use]
pub fn usage() -> String {
    let v = crate::consts::AGENT_VERSION;
    format!(
        "multitop-agent {v}\n\
         \n\
         Sampled by multitop over SSH. Prints one frame, or streams them.\n\
         \n\
         Usage:\n    \
           multitop-agent [monitor|docker|fetch|exec] [host] [cols] [lines] [cpu|mem]\n\
         \n\
         Modes:\n    \
           monitor   CPU, memory, network and the process table (the default)\n    \
           docker    the container table\n    \
           fetch     a one-shot host summary\n    \
           exec      run one command, reading the request from stdin\n\
         \n\
         Options:\n    \
           -h, --help       this text\n    \
           -V, --version    the version alone\n\
         \n\
         With stdout on a terminal it draws; piped, it writes packets for the\n\
         client to decode.\n"
    )
}

pub fn run_agent<I: IntoIterator<Item = String>>(raw: I) {
    use std::io::{self, IsTerminal, Write};
    use std::sync::atomic::Ordering;
    use std::time::{Duration, Instant};

    let args = parse_args(raw);
    if let Some(tell) = args.tell {
        let mut out = io::stdout().lock();
        let _ = match tell {
            Tell::Usage => write!(out, "{}", usage()),
            Tell::Version => writeln!(out, "multitop-agent {}", crate::consts::AGENT_VERSION),
            Tell::Unknown(what) => {
                let _ = writeln!(io::stderr(), "multitop-agent: {what}\n");
                write!(out, "{}", usage())
            }
        };
        return;
    }
    let is_tty = io::stdout().is_terminal();
    let pal = palette_for_env();
    let host = proc::host_info(args.display_ip.as_deref());
    let mut out = io::stdout().lock();

    match args.mode {
        Mode::Fetch => {
            let _ = emit_fetch(
                &fetch::sample_fetch(&host),
                args.cols,
                is_tty,
                pal,
                &mut out,
            );
        }
        Mode::Docker => {
            let _ = emit_docker(&host, docker::collect(), &args, is_tty, pal, &mut out);
        }
        Mode::Exec => exec::serve::serve(&host, args.cols, args.lines, &mut out),
        Mode::Monitor => {
            let stdin_gone = stdin_eof_watcher();
            if is_tty {
                let _ = out.write_all(b"\x1b[?25l");
            }
            let interval = Duration::from_secs_f64(DEFAULT_INTERVAL);
            let mut monitor = monitor::Monitor::new(host);
            let mut last = Instant::now();
            let mut next_tick = move || {
                std::thread::sleep(interval);
                if stdin_gone.load(Ordering::Relaxed) {
                    return None;
                }
                let elapsed = last.elapsed().as_secs_f64();
                last = Instant::now();
                Some(elapsed)
            };
            monitor_loop(&mut monitor, &args, is_tty, pal, &mut out, &mut next_tick);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Args {
        parse_args(v.iter().map(std::string::ToString::to_string))
    }

    #[test]
    fn defaults_without_arguments() {
        let a = args(&[]);
        assert_eq!(a.mode, Mode::Monitor);
        assert_eq!(a.display_ip, None);
        assert_eq!(a.cols, 80);
        assert_eq!(a.lines, 24);
    }

    #[test]
    fn full_monitor_invocation() {
        let a = args(&["monitor", "10.0.0.1", "120", "40"]);
        assert_eq!(a.mode, Mode::Monitor);
        assert_eq!(a.display_ip.as_deref(), Some("10.0.0.1"));
        assert_eq!(a.cols, 120);
        assert_eq!(a.lines, 40);
    }

    #[test]
    fn docker_mode_selected() {
        assert_eq!(args(&["docker", "10.0.0.1", "80", "24"]).mode, Mode::Docker);
    }

    #[test]
    fn mode_word_is_optional() {
        let a = args(&["10.0.0.1", "100", "30"]);
        assert_eq!(a.mode, Mode::Monitor);
        assert_eq!(a.display_ip.as_deref(), Some("10.0.0.1"));
        assert_eq!(a.cols, 100);
    }

    #[test]
    fn empty_ip_is_treated_as_absent() {
        assert_eq!(args(&["monitor", "", "90", "20"]).display_ip, None);
        assert_eq!(args(&["monitor", "", "90", "20"]).cols, 90);
    }

    #[test]
    fn malformed_dimensions_fall_back_to_defaults() {
        let a = args(&["monitor", "10.0.0.1", "wide", "tall"]);
        assert_eq!(a.cols, 80);
        assert_eq!(a.lines, 24);
    }

    #[test]
    fn partial_arguments_accepted() {
        let a = args(&["monitor", "10.0.0.1"]);
        assert_eq!(a.display_ip.as_deref(), Some("10.0.0.1"));
        assert_eq!(a.cols, 80);
    }
}
