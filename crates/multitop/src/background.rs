//! What colour is the terminal actually painting?
//!
//! The palette is a table of colour values and the terminal is a table of colour
//! values, and nothing in the chain between them knew they had to agree. Every one
//! of the eight themes is dark, so on a light terminal the text is drawn at
//! between 1.03:1 and 3.98:1 against its own background — `text` at 1.03:1 is not
//! a shade, it is the same colour as the page. The colours live in
//! [`multitop_agent::color`] and the fix is in [`multitop_agent::adapt`]; this
//! module is the half that answers the question the fix depends on.
//!
//! # Why a probe and not a setting
//!
//! The honest answer is that nobody knows their terminal's background, including
//! the person who chose it, so asking beats guessing. A setting is also worth
//! having, and the two are not in tension: detect, and let the setting override.
//!
//! # Why the probe must not hang
//!
//! `OSC 11` is a question, and a terminal that does not know the answer says
//! nothing at all rather than saying "no". So the read after the query is bounded,
//! and on expiry we fall through to the next probe. A terminal that answers
//! neither is the overwhelmingly common case — `COLORFGBG` is unset in most
//! emulators launched from a desktop, and not every terminal implements
//! `OSC 11` — and it must cost one bounded read, not a wedged startup.
//!
//! ## The bug that shape exists to prevent
//!
//! The first version checked the deadline in the `while` condition and then called
//! `stdin.read`, which **blocks**. A deadline checked around a blocking call is
//! not a timeout: the check does not run again until the read returns, and on a
//! terminal that never answers it does not return. That hung startup on every
//! terminal, and it got through the gate because the *release* binary the e2e
//! suite exercised was older than the probe — a stale artifact reading as a fix.
//! See [`read_reply`] for how the bound is now enforced by the call that waits.
//!
//! # What this cannot see
//!
//! A terminal whose background is a gradient or an image reports one flat colour,
//! and the palette is then legible against the lightest or darkest of it, not
//! against an average. Per-theme light variants have the same problem and worse:
//! they are wrong for every background but one. This is the better of the two
//! answers, not a complete one.

use std::io::Write;
use std::time::{Duration, Instant};

use multitop_agent::surface::Rgb;

/// The query: `OSC 11 ; ? BEL` — "what is your background colour?"
///
/// Written with the 7-bit BEL terminator rather than the 8-bit `ESC \`, because
/// the 8-bit form is not universally understood and a probe that some terminals
/// ignore is a probe that always times out.
pub const OSC11_QUERY: &str = "\x1b]11;?\x07";

/// How long to wait for the terminal to answer `OSC 11`.
///
/// 120 ms. A terminal that answers does so within a frame or two, and this is
/// paid once at startup. The alternative — a longer wait for a slower answer —
/// makes every launch on a terminal that cannot answer feel broken, and those
/// are the common case, not the rare one.
const PROBE_TIMEOUT: Duration = Duration::from_millis(120);

/// Hexadecimal, and how wide one digit of it is — the two facts the 16-bit reply
/// scaling is built from.
const RADIX: u32 = 16;
const BITS_PER_HEX_DIGIT: usize = 4;

/// The most hex digits a 16-bit colour component can carry.
const MAX_HEX_DIGITS: usize = 4;

/// Two hex digits per channel in the `#rrggbb` form, so six in all.
const HEX_BYTE_DIGITS: usize = 6;

/// A byte is 255, and this is arithmetic about bytes.
const CHANNEL_MAX_U32: u32 = 255;

/// The highest `COLORFGBG` index that means something.
///
/// 15 is the last of the 16 base colours. A larger index names a colour out of the
/// 256-cube, which depends on the terminal's own palette and so is a guess
/// rather than a measurement — and this probe exists to avoid guesses.
const MAX_COLOR_INDEX: u16 = 15;

/// The value `COLORFGBG` is known to hold when it is not useful.
///
/// Emulators that have never been configured leave it at this, and it reads as
/// "7;0" — light on dark — which would be a confident wrong answer. Detecting it
/// is the difference between falling back to the theme's own background and
/// adapting a dark terminal to a light palette because a string was set.
const DEFAULT_COLORFGBG: &str = "7;0";

/// Where the terminal's background came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Background {
    /// The terminal answered `OSC 11`.
    Reported(Rgb),
    /// `COLORFGBG` was set, and was not the unset default.
    Environment(Rgb),
    /// Nothing was known. The caller should use the theme's own background,
    /// which is the only background any theme states.
    Unknown,
}

impl Background {
    // reachability: called by the render path once adaptation is wired into
    // `App::current_theme`, which is the step that turns a stored Background into
    // an Adapted palette. Detection is live now — `boot_app` calls `detect` and
    // stores the result — but nothing consumes it yet, so the value is recorded
    // rather than dropped. 26 places take `&Palette` and the renderers read
    // `pal.reset` as a FIELD, so that step is a two-crate signature change.
    /// The colour to adapt against, given what the theme thinks its own is.
    ///
    /// `theme_default` is [`multitop_agent::color::Palette::own_background`]:
    /// every theme paints a keybar strip and nothing else, so that value is the
    /// only statement any theme makes about what is behind it.
    #[must_use]
    pub const fn or_theme(self, theme_default: Rgb) -> Rgb {
        match self {
            Self::Reported(colour) | Self::Environment(colour) => colour,
            Self::Unknown => theme_default,
        }
    }

    /// Which probe answered, for the log line and for tests.
    #[must_use]
    pub const fn source(self) -> &'static str {
        match self {
            Self::Reported(_) => "OSC 11",
            Self::Environment(_) => "COLORFGBG",
            Self::Unknown => "unknown",
        }
    }
}

/// The narrow seam over "a terminal that might answer".
///
/// A trait rather than a direct read, because the interesting cases — a terminal
/// that answers, one that stays silent, one that answers with nonsense — cannot be
/// produced reliably on demand otherwise, and an untestable probe has a timeout
/// path that has never run.
pub trait Tty {
    /// Write `query`, and return whatever came back before `timeout` if anything
    /// did.
    fn query(&mut self, query: &str, timeout: Duration) -> Option<String>;
}

/// Read the background: `OSC 11` first, then `COLORFGBG`.
///
/// The environment lookup is a parameter, not `std::env`, so the fallback path is
/// testable — a probe whose fallback is not exercised is a fallback that has
/// never run until the day it mattered.
pub fn detect_with(env: &dyn Fn(&str) -> Option<String>, tty: &mut dyn Tty) -> Background {
    if let Some(reply) = tty.query(OSC11_QUERY, PROBE_TIMEOUT) {
        if let Some(colour) = parse_osc11_reply(&reply) {
            return Background::Reported(colour);
        }
    }
    if let Some(value) = env("COLORFGBG") {
        if let Some(colour) = parse_colorfgbg(&value) {
            return Background::Environment(colour);
        }
    }
    Background::Unknown
}

/// Detect the background of the real terminal.
#[must_use]
pub fn detect() -> Background {
    let env = |key: &str| std::env::var(key).ok();
    let mut tty = RealTty;
    detect_with(&env, &mut tty)
}

/// A `Tty` over the process's own terminal.
///
/// No state: the query goes to stdout and the answer comes back through crossterm
/// from the process's own stdin, so there is nothing to hold between the two.
struct RealTty;

impl Tty for RealTty {
    fn query(&mut self, query: &str, timeout: Duration) -> Option<String> {
        // A terminal only answers a question addressed to its display, so this
        // needs a real tty on both ends. Without one there is nothing to ask, and
        // the caller falls through to the environment.
        if !std::io::IsTerminal::is_terminal(&std::io::stdout())
            || !std::io::IsTerminal::is_terminal(&std::io::stdin())
        {
            return None;
        }
        let mut out = std::io::stdout();
        if out.write_all(query.as_bytes()).is_err() || out.flush().is_err() {
            return None;
        }
        read_reply(timeout)
    }
}

/// Read the reply, bounded by the call that waits for it.
///
/// `crossterm::event::poll` is already a direct dependency and owns the `poll(2)`
/// FFI, which matters twice over: this crate has `#![deny(unsafe_code)]` and a
/// policy of no FFI of its own, so the honest way to ask "is there input yet" is
/// to borrow the crate that already asks it. The `unsafe` that a direct
/// `libc::poll` needs would have made this the only unsafe file in `multitop`,
/// and `check_unsafe_scope.py` correctly refused it.
///
/// A deadline checked around `stdin.read` is what this replaces, and the
/// difference is the whole point: `read` blocks, so a check in the loop condition
/// never runs again. Here the wait is bounded by `poll` and the read only happens
/// once there is something to read.
fn read_reply(timeout: Duration) -> Option<String> {
    let deadline = Instant::now() + timeout;
    let mut collected = String::new();
    while Instant::now() < deadline {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if !crossterm::event::poll(remaining).unwrap_or(false) {
            return None;
        }
        match crossterm::event::read() {
            Err(_) => return None,
            Ok(crossterm::event::Event::Key(crossterm::event::KeyEvent {
                code: crossterm::event::KeyCode::Char(character),
                ..
            })) => collected.push(character),
            Ok(crossterm::event::Event::Paste(text)) => collected.push_str(&text),
            Ok(_) => {}
        }
        // `ST` is ESC backslash and BEL ends the 7-bit form. Either ends the
        // reply; anything after it belongs to whatever the user is doing next,
        // and swallowing it would eat a keystroke.
        if collected.ends_with("\x1b\\") || collected.ends_with('\x07') {
            return Some(collected);
        }
    }
    None
}

/// Parse the reply to `OSC 11`.
///
/// The shape is `ESC ] 11 ; rgb:RRRR/GGGG/BBBB ST`, and the components are 1–4
/// hex digits each, scaled to 16 bits by the reply rather than to 8. Some
/// terminals answer `#rrggbb` instead, so that is handled rather than refused.
#[must_use]
pub fn parse_osc11_reply(reply: &str) -> Option<Rgb> {
    let body = reply
        .trim_end_matches('\x07')
        .trim_end_matches("\x1b\\")
        .trim();
    // Drop the `ESC ] 11 ;` prefix. The terminal may have echoed other input
    // first, so the sequence is found rather than assumed to be at the start.
    let value = body.split_once(';')?.1.trim();
    if let Some(hex) = value.strip_prefix("rgb:") {
        let parts: Vec<&str> = hex.split('/').collect();
        if parts.len() != 3 {
            return None;
        }
        let mut out = [0_u8; 3];
        for (slot, part) in out.iter_mut().zip(parts.iter()) {
            *slot = scale_hex16(part)?;
        }
        return Some(Rgb::new(out[0], out[1], out[2]));
    }
    // `#rrggbb`: two digits per channel, so the bytes ARE the value -- no
    // scaling, unlike the `rgb:` form above.
    if let Some(hex) = value.strip_prefix('#') {
        if hex.len() != HEX_BYTE_DIGITS {
            return None;
        }
        let mut out = [0_u8; 3];
        for (slot, pair) in out.iter_mut().zip(hex.as_bytes().chunks(2)) {
            *slot = u8::from_str_radix(std::str::from_utf8(pair).ok()?, RADIX).ok()?;
        }
        return Some(Rgb::new(out[0], out[1], out[2]));
    }
    None
}

/// Scale a 1–4 digit hex component to a byte.
///
/// The reply is 16-bit per component, so `ffff` is full white and `0000` is black;
/// a terminal that sends fewer digits is sending a coarser value, and the scale
/// depends on the width. Dividing by the maximum for the width present is the
/// only reading under which `f` and `ffff` are the same colour.
fn scale_hex16(component: &str) -> Option<u8> {
    if component.is_empty() || component.len() > MAX_HEX_DIGITS {
        return None;
    }
    if !component.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let value = u32::from_str_radix(component, RADIX).ok()?;
    let max = (1_u32 << (component.len() * BITS_PER_HEX_DIGIT)) - 1;
    // The division bounds this at a byte, so the conversion cannot truncate; the
    // `try_from` is what says so rather than leaving a reader to work it out.
    u8::try_from((value * CHANNEL_MAX_U32) / max).ok()
}

/// Parse `COLORFGBG`, whose format is `fg;bg` with 0–16 indices.
///
/// Only the second field matters. Index 0 is black, 7 is light grey, 8 and above
/// are bright. The unset default is refused, because treating it as a background
/// would adapt a dark terminal to a light palette on the strength of a string an
/// emulator set without being asked.
#[must_use]
pub fn parse_colorfgbg(value: &str) -> Option<Rgb> {
    let value = value.trim();
    if value == DEFAULT_COLORFGBG {
        return None;
    }
    // Exactly two numeric fields. Taking the last field regardless is how a lone
    // "0" became a confident black background, and a variable holding one number
    // is not the documented shape at all -- it is something else that happens to
    // be set.
    let mut fields = value.split(';');
    let _foreground = fields.next()?;
    let background = fields.next()?.trim().parse::<u16>().ok()?;
    if fields.next().is_some() {
        return None;
    }
    if background > MAX_COLOR_INDEX {
        return None;
    }
    Some(index_to_rgb(background))
}

/// The 16 ANSI base colours, in order. Not the full 256-colour cube: the first
/// 16 are the ones every terminal agrees on, and a higher index is a guess.
fn index_to_rgb(index: u16) -> Rgb {
    /// The xterm defaults, which are what indices 0–15 have meant since 1984.
    const CUBE: [[u8; 3]; 16] = [
        [0, 0, 0],
        [205, 0, 0],
        [0, 205, 0],
        [205, 205, 0],
        [0, 0, 238],
        [205, 0, 205],
        [0, 205, 205],
        [229, 229, 229],
        [127, 127, 127],
        [255, 0, 0],
        [0, 255, 0],
        [255, 255, 0],
        [92, 92, 255],
        [255, 0, 255],
        [0, 255, 255],
        [255, 255, 255],
    ];
    let entry = CUBE[usize::from(index)];
    Rgb::new(entry[0], entry[1], entry[2])
}

#[cfg(test)]
#[path = "background_tests.rs"]
mod tests;
