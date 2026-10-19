//! The log the panel holds, against what a terminal emulator shows.
//!
//! The painter is a line model of a terminal, and every defect it has had was
//! a place where the model and a terminal disagree: a `\r` read as a line, an
//! `ESC[nA` read as text, apt's pinned progress bar -- `ESC 7`, an absolute
//! move to the bottom row, the bar, `ESC 8` -- flattened onto the next log
//! line. Each was found by a user, one case at a time. This asks the question
//! once, for a corpus: feed the same bytes to the real painter, through the
//! real `Msg` path into a real `App`, and to `vt100`, and after every line
//! compare the panel's log with the emulator's scroll region and the panel's
//! status line with the row outside it.
//!
//! Two concessions to the oracle, both about the oracle and neither about the
//! painter. `vt100` 0.16.2 does not implement HVP (`ESC[r;cf`), which ECMA-48
//! defines as identical to CUP, so the oracle's copy has `f` rewritten to `H`.
//! And it keeps no scrollback while a scroll region is set, so the comparison
//! is of the visible region after every line, never of history -- which is
//! the stronger check anyway, because a tear shows up the moment it happens.
//! The emulator is wide (512 columns) because the painter does not model
//! columns: a long line is one log line and would be two wrapped rows at 80.

use multitop::app::{App, Mode};
use multitop::config::Server;
use multitop::panel::UpgradeState;
use multitop::tasks::{painted_msg, Painter};
use multitop_agent::color::strip_ansi;
use std::num::NonZeroU16;

// vt100 0.18 takes the size as NonZeroU16: a zero-sized screen is unrepresentable.
const ROWS: NonZeroU16 = NonZeroU16::new(24).unwrap();
const COLS: NonZeroU16 = NonZeroU16::new(512).unwrap();

/// apt 3.0 installing two packages in a Debian 13 container, captured from a
/// 24x80 pty with `script` (2026-10-10). Real bytes, not a reconstruction:
/// the download progress, dpkg's `(Reading database ... 5%` rewrites, the
/// scroll region, the pinned bar, and its teardown.
const APT: &[u8] = include_bytes!("fixtures/apt-install-24x80.bytes");

/// `docker compose pull`: a block of layers redrawn by moving up over it.
const DOCKER: &[u8] = b"a Pulling\r\nb Pulling\r\n\
\x1b[2A\x1b[2Ka Downloading 10%\r\n\x1b[2Kb Waiting\r\n\
\x1b[2A\x1b[2Ka Downloading 80%\r\n\x1b[2Kb Downloading 5%\r\n\
\x1b[2A\x1b[2Ka Pull complete\r\n\x1b[2Kb Pull complete\r\n\
done\r\n";

/// A Rich live display: drawn, then rewound from its own last line and drawn
/// again -- twice at the same height, then shorter, then taller than the rows
/// above it, so it erases a line it did not draw.
const RICH: &[u8] = b"\x1b[?25lhead\r\n+--+\r\n|1 |\r\n+--+\
\r\x1b[2K\x1b[1A\x1b[2K\x1b[1A\x1b[2K+--+\r\n|2 |\r\n+--+\
\r\x1b[2K\x1b[1A\x1b[2K\x1b[1A\x1b[2K|3 |\
\r\x1b[2K\x1b[1A\x1b[2K|4 |\r\n|4b|\r\n\x1b[?25htail\r\n";

/// curl's meter: one line rewritten with `\r`, each state as long as the last.
const CURL: &[u8] = b"  % Total\r\n  0  100\r 50  100\r100  100\r\nok\r\n";

/// `tput sgr0` ends in `ESC(B`, a charset designation, before the SGR reset.
const SGR0: &[u8] = b"\x1b[1mbold\x1b(B\x1b[m plain\r\nnext\x1b(B\x1b[m line\r\n";

fn app() -> App {
    let mut app = App::new(vec![Server {
        upgrade_cmd: Some("true".to_string()),
        ..crate::common::local_server("127.0.0.1")
    }]);
    app.panels[0].upgrade_state = UpgradeState::STARTED;
    app.panels[0].mode = Mode::Upgrade;
    app
}

fn plain(line: &str) -> String {
    strip_ansi(line).trim_end().to_string()
}

fn without_trailing_blanks(mut rows: Vec<String>) -> Vec<String> {
    while rows.last().is_some_and(String::is_empty) {
        rows.pop();
    }
    rows
}

/// HVP is CUP in ECMA-48; `vt100` implements only the latter.
fn for_oracle(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut csi_params = false;
    for (i, &b) in bytes.iter().enumerate() {
        if csi_params && b == b'f' {
            out.push(b'H');
            csi_params = false;
            continue;
        }
        if csi_params && !(b.is_ascii_digit() || b == b';') {
            csi_params = false;
        }
        if b == b'[' && i > 0 && bytes[i - 1] == 0x1b {
            csi_params = true;
        }
        out.push(b);
    }
    out
}

/// Feed `bytes` one line at a time to both, and compare after each line.
fn agree(name: &str, bytes: &[u8]) {
    compare(name, bytes, bytes);
}

/// `agree`, with the painter and the emulator fed separately -- line `n` of
/// each together -- so the comparison itself can be calibrated.
fn compare(name: &str, painter_bytes: &[u8], bytes: &[u8]) {
    let mut app = app();
    let gen = app.panels[0].upgrade_gen;
    let mut painter = Painter::new();
    let mut term = vt100::Parser::new(ROWS, COLS, 0);
    // Rows the emulator's scroll region covers; apt sets `ESC[0;23r`, so its
    // last row is the pinned one.
    let mut region = usize::from(ROWS.get());

    let painter_chunks: Vec<&[u8]> = painter_bytes.split_inclusive(|&b| b == b'\n').collect();
    for (n, chunk) in bytes.split_inclusive(|&b| b == b'\n').enumerate() {
        for painted in painter.feed_bytes(painter_chunks[n]) {
            app.apply(painted_msg(0, gen, painted, std::convert::identity));
        }
        term.process(&for_oracle(chunk));
        if chunk.windows(4).any(|w| w == b";23r") {
            region = usize::from(ROWS.get()) - 1;
        }
        if chunk.windows(4).any(|w| w == b";24r") {
            region = usize::from(ROWS.get());
        }

        let screen: Vec<String> = term
            .screen()
            .rows(0, COLS.get())
            .map(|r| plain(&r))
            .collect();
        let shown = without_trailing_blanks(screen[..region].to_vec());
        let log = without_trailing_blanks(
            app.panels[0]
                .last_upgrade
                .iter()
                .map(|l| plain(l))
                .collect(),
        );
        assert!(
            log.len() >= shown.len() && log[log.len() - shown.len()..] == shown[..],
            "{name}, after line {n} ({:?}):\n  the panel's log ends\n{}\n  the terminal's scroll region shows\n{}",
            String::from_utf8_lossy(chunk),
            log.iter().rev().take(shown.len()).rev().map(|l| format!("    |{l}|")).collect::<Vec<_>>().join("\n"),
            shown.iter().map(|l| format!("    |{l}|")).collect::<Vec<_>>().join("\n"),
        );

        let status = app.panels[0]
            .upgrade_status
            .as_ref()
            .filter(|(g, _)| *g == gen)
            .map(|(_, s)| plain(s));
        if region < usize::from(ROWS.get()) || status.is_some() {
            assert_eq!(
                status.unwrap_or_default(),
                screen[usize::from(ROWS.get()) - 1],
                "{name}, after line {n}: the panel's status line against the terminal's bottom row"
            );
        }
    }
    for painted in painter.finish() {
        app.apply(painted_msg(0, gen, painted, std::convert::identity));
    }
    assert_eq!(
        app.panels[0].upgrade_status, None,
        "{name}: a finished run leaves no status line up"
    );
}

#[test]
fn apt_installing_agrees_with_a_terminal() {
    agree("apt", APT);
}

#[test]
fn a_docker_layer_block_agrees_with_a_terminal() {
    agree("docker", DOCKER);
}

#[test]
fn a_rich_live_display_agrees_with_a_terminal() {
    agree("rich", RICH);
}

#[test]
fn a_curl_meter_agrees_with_a_terminal() {
    agree("curl", CURL);
}

#[test]
fn tput_sgr0_agrees_with_a_terminal() {
    agree("sgr0", SGR0);
}

/// The oracle itself, calibrated: it must be able to say no. A painter that
/// flattened apt's bar into the log -- the defect this file exists for --
/// is simulated by feeding the bar to the app as a log line.
#[test]
fn the_oracle_rejects_a_flattened_status_line() {
    let result = std::panic::catch_unwind(|| {
        // The bracket and the move removed, the bar kept as text: the old
        // painter's reading, reproduced in the bytes the painter is given.
        // The emulator still sees the real stream.
        let flattened = String::from_utf8_lossy(APT)
            .replace("\x1b7", "")
            .replace("\x1b8", "")
            .replace("\x1b[24;0f", "");
        compare("apt, flattened", flattened.as_bytes(), APT);
    });
    assert!(
        result.is_err(),
        "the comparison could not tell a torn log from a clean one"
    );
}
