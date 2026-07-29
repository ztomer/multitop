//! Compact system and Docker monitor.
//!
//! Built as a small static binary that `multitop` uploads to each monitored
//! host and runs over SSH. It writes frames to stdout; when stdout is a
//! terminal it repaints in place, otherwise it delimits frames with
//! `===MONITOR===` so the reader can tell them apart.

pub mod color;
pub mod consts;
pub mod docker;
pub mod docker_cli;
pub mod docker_render;
pub mod fetch;
pub mod fmt;
pub mod monitor;
pub mod proc;
pub mod proc_sys;
pub mod proto;
pub mod render;
pub mod render_layout;
pub mod sys;

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
    pub fn word(&self) -> &'static str {
        match self {
            SortBy::Cpu => "cpu",
            SortBy::Mem => "mem",
        }
    }
}

/// Parsed command line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Args {
    pub mode: Mode,
    pub display_ip: Option<String>,
    pub cols: usize,
    pub lines: usize,
    pub sort: SortBy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Monitor,
    Docker,
    Fetch,
}

impl Mode {
    pub fn word(&self) -> &'static str {
        match self {
            Mode::Monitor => "monitor",
            Mode::Docker => "docker",
            Mode::Fetch => "fetch",
        }
    }
}

impl Default for Args {
    fn default() -> Self {
        Args {
            mode: Mode::Monitor,
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
            "fetch" => {
                args.mode = Mode::Fetch;
                rest.remove(0);
            }
            _ => {}
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
        Mode::Fetch => {
            let snap = fetch::sample_fetch(&host);
            if is_tty {
                // Text-only fallback — the full rendering with logos lives in
                // the monitor crate's fetch_render module.
                let details = [
                    ("OS", &snap.os),
                    ("Kernel", &snap.kernel),
                    ("Uptime", &snap.uptime),
                    ("Host", &snap.host_model),
                    ("CPU", &snap.cpu_model),
                    ("Memory", &snap.memory_str),
                    ("Disk", &snap.disk_str),
                ];
                let mut out = io::stdout().lock();
                let _ = writeln!(
                    out,
                    "{}",
                    crate::fmt::center_header(&snap.user_host, args.cols, pal)
                );
                for (label, val) in &details {
                    let _ = writeln!(
                        out,
                        "  {}{:<7}{}: {}{}{}",
                        pal.bold, label, pal.reset, pal.white, val, pal.reset
                    );
                }
            } else {
                let payload = proto::Payload::Fetch(snap);
                let bytes = proto::encode_packet(&payload);
                let mut out = io::stdout().lock();
                let _ = out.write_all(&bytes);
                let _ = out.flush();
            }
        }
        Mode::Docker => {
            let rows = docker::collect();
            if is_tty {
                let frame = docker::render(&host, args.cols, args.lines, &rows, pal, args.sort);
                let mut out = io::stdout().lock();
                let _ = writeln!(out, "{}", frame.join("\n"));
            } else {
                let payload = proto::Payload::Docker { host, rows };
                let bytes = proto::encode_packet(&payload);
                let mut out = io::stdout().lock();
                let _ = out.write_all(&bytes);
                let _ = out.flush();
            }
        }
        Mode::Monitor => {
            if is_tty {
                let _ = io::stdout().write_all(b"\x1b[?25l");
            }
            let interval = Duration::from_secs_f64(DEFAULT_INTERVAL);
            let mut monitor = monitor::Monitor::new(host);

            let mut buf = String::with_capacity(8192);
            let mut last = Instant::now();

            // Emit initial frame immediately so client connection is instant (<10ms)
            let snap = monitor.tick(0.0, 120, 50, args.sort);
            if is_tty {
                buf.push_str("\x1b[H\x1b[J");
                render::render_to_buf(
                    &snap,
                    args.cols,
                    args.lines,
                    render::bar_len_for(args.cols),
                    pal,
                    &mut buf,
                );
                let mut out = io::stdout().lock();
                let _ = out.write_all(buf.as_bytes());
                let _ = out.flush();
            } else {
                let payload = proto::Payload::Monitor(snap);
                let bytes = proto::encode_packet(&payload);
                let mut out = io::stdout().lock();
                if out.write_all(&bytes).is_err() || out.flush().is_err() {
                    return;
                }
            }

            std::thread::sleep(interval);

            loop {
                let elapsed = last.elapsed().as_secs_f64();
                last = Instant::now();

                if is_tty {
                    let snap = monitor.tick(elapsed, args.cols, args.lines, args.sort);
                    buf.clear();
                    buf.push_str("\x1b[H\x1b[J");
                    render::render_to_buf(
                        &snap,
                        args.cols,
                        args.lines,
                        render::bar_len_for(args.cols),
                        pal,
                        &mut buf,
                    );
                    let mut out = io::stdout().lock();
                    if out.write_all(buf.as_bytes()).is_err() || out.flush().is_err() {
                        return;
                    }
                } else {
                    // In binary stream mode (over SSH), sample up to 50 top procs.
                    // The client dashboard will filter/render as many as fit locally.
                    let snap = monitor.tick(elapsed, 120, 50, args.sort);
                    let payload = proto::Payload::Monitor(snap);
                    let bytes = proto::encode_packet(&payload);
                    let mut out = io::stdout().lock();
                    if out.write_all(&bytes).is_err() || out.flush().is_err() {
                        return;
                    }
                }

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
