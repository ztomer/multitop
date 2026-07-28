//! Compact system and Docker monitor.
//!
//! Built as a small static binary that `multitop` uploads to each monitored
//! host and runs over SSH. It writes frames to stdout; when stdout is a
//! terminal it repaints in place, otherwise it delimits frames with
//! `===MONITOR===` so the reader can tell them apart.

pub mod color;
pub mod consts;
pub mod docker;
pub mod fmt;
pub mod monitor;
pub mod proc;
pub mod render;

/// Frame delimiter on the wire between agent and TUI.
pub const FRAME_MARKER: &str = "===MONITOR===";

/// Default refresh interval, in seconds.
pub const DEFAULT_INTERVAL: f64 = 2.0;

/// Parsed command line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Args {
    pub mode: Mode,
    pub display_ip: Option<String>,
    pub cols: usize,
    pub lines: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Monitor,
    Docker,
}

impl Default for Args {
    fn default() -> Self {
        Args {
            mode: Mode::Monitor,
            display_ip: None,
            cols: 80,
            lines: 24,
        }
    }
}

/// Parse `[monitor|docker] [ip] [cols] [lines]`.
///
/// The mode word is optional so the binary still behaves sensibly when run by
/// hand on a server with no arguments at all.
pub fn parse_args<I: IntoIterator<Item = String>>(argv: I) -> Args {
    let mut args = Args::default();
    let mut rest: Vec<String> = argv.into_iter().collect();

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
            _ => {}
        }
    }

    let mut positional = rest.into_iter();
    args.display_ip = positional.next().filter(|s| !s.is_empty());
    // A malformed dimension falls back to the default rather than aborting;
    // a wrong panel size is far better than no panel.
    if let Some(v) = positional.next().and_then(|v| v.parse().ok()) {
        args.cols = v;
    }
    if let Some(v) = positional.next().and_then(|v| v.parse().ok()) {
        args.lines = v;
    }
    args
}

pub fn run_agent<I: IntoIterator<Item = String>>(argv: I) {
    use std::io::{self, IsTerminal, Write};
    use std::time::{Duration, Instant};

    let args = parse_args(argv);
    let is_tty = io::stdout().is_terminal();
    let pal = if std::env::var_os("NO_COLOR").is_some() {
        &color::PLAIN
    } else {
        &color::ANSI
    };
    let host = proc::host_info(args.display_ip.as_deref());

    match args.mode {
        Mode::Docker => {
            let frame = docker::render(&host, args.cols, args.lines, &docker::collect(), pal);
            let mut out = io::stdout().lock();
            let _ = writeln!(out, "{}", frame.join("\n"));
        }
        Mode::Monitor => {
            if is_tty {
                let _ = io::stdout().write_all(b"\x1b[?25l");
            }
            let interval = Duration::from_secs_f64(DEFAULT_INTERVAL);
            let mut monitor = monitor::Monitor::new(host);

            let mut buf = String::with_capacity(8192);
            let mut last = Instant::now();
            std::thread::sleep(interval);

            loop {
                let elapsed = last.elapsed().as_secs_f64();
                last = Instant::now();

                let snap = monitor.tick(elapsed, args.cols, args.lines);
                let frame = render::render(&snap, args.cols, args.lines, render::bar_len_for(args.cols), pal);

                buf.clear();
                if is_tty {
                    buf.push_str("\x1b[H\x1b[J");
                } else {
                    buf.push_str(FRAME_MARKER);
                    buf.push('\n');
                }
                for line in &frame {
                    buf.push_str(line);
                    buf.push('\n');
                }

                let mut out = io::stdout().lock();
                if out.write_all(buf.as_bytes()).is_err() || out.flush().is_err() {
                    return;
                }
                drop(out);

                std::thread::sleep(interval);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Args {
        parse_args(v.iter().map(|s| s.to_string()))
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
