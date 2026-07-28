use std::io::{self, IsTerminal, Write};
use std::time::{Duration, Instant};

use multitop_agent::color::{Palette, ANSI, PLAIN};
use multitop_agent::monitor::Monitor;
use multitop_agent::render::{bar_len_for, render};
use multitop_agent::{docker, parse_args, proc, Mode, DEFAULT_INTERVAL, FRAME_MARKER};

fn main() {
    let args = parse_args(std::env::args().skip(1));
    let is_tty = io::stdout().is_terminal();
    // Colors stay on over a pipe: the TUI reads these SGR codes and converts
    // them into its own styling.
    let pal = if std::env::var_os("NO_COLOR").is_some() {
        &PLAIN
    } else {
        &ANSI
    };
    let host = proc::host_info(args.display_ip.as_deref());

    match args.mode {
        Mode::Docker => {
            let frame = docker::render(&host, args.cols, &docker::collect(), pal);
            let mut out = io::stdout().lock();
            let _ = writeln!(out, "{}", frame.join("\n"));
        }
        Mode::Monitor => {
            if is_tty {
                let _ = io::stdout().write_all(b"\x1b[?25l"); // hide cursor
            }
            run_monitor(host, args.cols, args.lines, is_tty, pal);
            if is_tty {
                let _ = io::stdout().write_all(b"\x1b[?25h");
                let _ = io::stdout().flush();
            }
        }
    }
}

fn run_monitor(host: String, cols: usize, lines: usize, is_tty: bool, pal: &Palette) {
    let interval = Duration::from_secs_f64(DEFAULT_INTERVAL);
    // Reads the baseline counters; the first frame is a real delta, not zeroes.
    let mut monitor = Monitor::new(host);

    let mut buf = String::with_capacity(8192);
    let mut last = Instant::now();
    std::thread::sleep(interval);

    loop {
        // Measure the window actually slept rather than assuming it, so a
        // descheduled agent reports correct rates instead of inflated ones.
        let elapsed = last.elapsed().as_secs_f64();
        last = Instant::now();

        let snap = monitor.tick(elapsed, cols, lines);
        let frame = render(&snap, cols, bar_len_for(cols), pal);

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

        // A closed pipe means the TUI went away; exit quietly rather than
        // leaving an orphan looping on a dead descriptor.
        let mut out = io::stdout().lock();
        if out.write_all(buf.as_bytes()).is_err() || out.flush().is_err() {
            return;
        }
        drop(out);

        std::thread::sleep(interval);
    }
}
