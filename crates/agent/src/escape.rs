//! One reading of the terminal escape grammar, for everything that has to tell
//! a tool's text from its control sequences.
//!
//! # Why this exists
//!
//! There were three readers, each hand-rolled and each knowing a different
//! subset of ECMA-48. The upgrade painter consumed `ESC[` and nothing else; the
//! span renderer dropped the ESC of any non-CSI escape and printed the byte
//! after it; `strip_ansi` swallowed one byte after an ESC. apt draws its
//! progress bar with `ESC 7`, `ESC[<row>;0f`, the bar, `ESC 8` and no newline,
//! so the painter glued the bar onto the next log line and the renderer framed
//! it with a literal `7` and `8` -- the upgrade log tore on every package.
//!
//! The fix for a family of partial parsers is not a fourth, slightly larger
//! one. This wraps `vte`'s implementation of the DEC state machine
//! (<https://vt100.net/emu/dec_ansi_parser>), so every sequence is consumed
//! whole -- two-byte escapes, charset designations, every CSI final byte, OSC
//! and DCS strings with either terminator -- and each reader only decides what
//! the sequences it cares about MEAN.

use vte::{Params, Parser, Perform};

/// One unit of a byte stream, as a terminal would read it.
#[derive(Debug)]
pub enum Token<'a> {
    /// A printable character.
    Print(char),
    /// A C0 or C1 control: `\r`, `\n`, `\t`, BEL, backspace.
    Control(u8),
    /// A control sequence, `ESC [` through its final byte.
    Csi(Csi<'a>),
    /// Any other escape: `ESC 7`, `ESC 8`, `ESC M`, `ESC ( B`, the `ESC \`
    /// that terminates a string.
    Esc {
        /// Intermediate bytes, e.g. the `(` of a charset designation.
        intermediates: &'a [u8],
        /// The final byte.
        byte: u8,
    },
    /// An operating-system command, `ESC ]` through BEL or `ESC \\`, split at
    /// its `;`s: `ESC]11;rgb:ffff/ffff/ffff BEL` is `[b"11", b"rgb:ffff/ffff/ffff"]`.
    Osc(&'a [&'a [u8]]),
    /// A sequence consumed whole that carries no text and no meaning a reader
    /// here acts on: DCS, and a CSI too long for the parser to hold.
    Opaque,
}

/// A parsed control sequence.
#[derive(Debug)]
pub struct Csi<'a> {
    params: &'a Params,
    /// Private markers and intermediates: the `?` of `ESC[?25l`.
    pub intermediates: &'a [u8],
    /// The final byte: `m` for colour, `A` for cursor up.
    pub action: char,
}

impl Csi<'_> {
    /// Whether this is Select Graphic Rendition -- colour and weight, the only
    /// sequence that is part of how a line LOOKS rather than where it goes.
    #[must_use]
    pub const fn is_sgr(&self) -> bool {
        self.action == 'm' && self.intermediates.is_empty()
    }

    /// Every parameter and subparameter, in order. `38;2;1;2;3` and
    /// `38:2:1:2:3` both read as `[38, 2, 1, 2, 3]`.
    pub fn flat(&self) -> impl Iterator<Item = u16> + '_ {
        self.params.iter().flat_map(|p| p.iter().copied())
    }

    /// SGR parameters, flat, with the colon form of an extended colour
    /// rewritten to the semicolon form every SGR reader already knows.
    ///
    /// `38:2::1:2:3` (ITU T.416, with its empty colour-space slot) and
    /// `38:2:1:2:3` both become `38;2;1;2;3`, and `38:5:196` becomes
    /// `38;5;196`. Flattening the colon form naively reads the colour-space
    /// slot as red and shifts every channel by one.
    #[must_use]
    pub fn sgr_params(&self) -> Vec<u16> {
        let mut out = Vec::new();
        for group in self.params {
            match group {
                [kind @ (38 | 48 | 58), 2, .., r, g, b] => out.extend([*kind, 2, *r, *g, *b]),
                [kind @ (38 | 48 | 58), 5, n, ..] => out.extend([*kind, 5, *n]),
                _ => out.extend_from_slice(group),
            }
        }
        out
    }

    /// The first parameter, `0` when absent.
    #[must_use]
    pub fn first(&self) -> u16 {
        self.params
            .iter()
            .next()
            .and_then(|p| p.first().copied())
            .unwrap_or(0)
    }

    /// The first parameter as a count, with a terminal's defaulting: absent or
    /// `0` both mean one (`ESC[A` and `ESC[0A` each move up one row).
    #[must_use]
    pub fn count(&self) -> usize {
        usize::from(self.first().max(1))
    }

    /// The sequence written back out, in canonical form.
    ///
    /// Used to keep SGR inside a painted line for the renderer to read; the
    /// form may differ from the original bytes (`ESC[m` comes back `ESC[0m`)
    /// but never the meaning.
    pub fn write_to(&self, out: &mut String) {
        out.push_str("\x1b[");
        for &b in self
            .intermediates
            .iter()
            .filter(|b| (0x3C..=0x3F).contains(*b))
        {
            out.push(char::from(b));
        }
        for (i, param) in self.params.iter().enumerate() {
            if i > 0 {
                out.push(';');
            }
            for (j, sub) in param.iter().enumerate() {
                if j > 0 {
                    out.push(':');
                }
                out.push_str(&sub.to_string());
            }
        }
        for &b in self
            .intermediates
            .iter()
            .filter(|b| !(0x3C..=0x3F).contains(*b))
        {
            out.push(char::from(b));
        }
        out.push(self.action);
    }
}

struct Visitor<F>(F);

impl<F: FnMut(Token<'_>)> Perform for Visitor<F> {
    fn print(&mut self, c: char) {
        (self.0)(Token::Print(c));
    }

    fn execute(&mut self, byte: u8) {
        (self.0)(Token::Control(byte));
    }

    fn csi_dispatch(&mut self, params: &Params, intermediates: &[u8], ignore: bool, action: char) {
        if ignore {
            (self.0)(Token::Opaque);
        } else {
            (self.0)(Token::Csi(Csi {
                params,
                intermediates,
                action,
            }));
        }
    }

    fn esc_dispatch(&mut self, intermediates: &[u8], _ignore: bool, byte: u8) {
        (self.0)(Token::Esc {
            intermediates,
            byte,
        });
    }

    fn osc_dispatch(&mut self, params: &[&[u8]], _bell_terminated: bool) {
        (self.0)(Token::Osc(params));
    }

    fn unhook(&mut self) {
        (self.0)(Token::Opaque);
    }
}

/// Read `bytes` as a terminal would, calling `visit` once per token.
///
/// A sequence cut off by the end of `bytes` produces no token: it is not
/// finished, and its bytes are neither text nor control yet. [`is_complete`]
/// says whether that happened.
pub fn scan(bytes: &[u8], visit: impl FnMut(Token<'_>)) {
    let mut parser = Parser::new();
    parser.advance(&mut Visitor(visit), bytes);
}

/// Whether `bytes` ends at a token boundary -- not inside an escape sequence,
/// a string, or a multi-byte character.
///
/// Probed by feeding a sentinel after the bytes, in its own `advance`: from a
/// boundary, `x` is exactly one token, itself. From inside a sequence it
/// becomes the sequence's final byte or part of its body; from inside a UTF-8
/// character it first flushes a replacement glyph. (In the SAME `advance` call
/// the cut character and the sentinel are decoded together as one invalid run,
/// which ends in a clean `x` -- measured, which is why the probe is separate.)
#[must_use]
pub fn is_complete(bytes: &[u8]) -> bool {
    let mut parser = Parser::new();
    parser.advance(&mut Visitor(|_: Token<'_>| {}), bytes);
    let mut tokens = 0usize;
    let mut sentinel = false;
    parser.advance(
        &mut Visitor(|t: Token<'_>| {
            tokens += 1;
            sentinel = matches!(t, Token::Print('x'));
        }),
        b"x",
    );
    tokens == 1 && sentinel
}

/// The text of `s` with every escape sequence removed. Control characters
/// (`\n`, `\t`, `\r`) are kept: they are layout, and callers that measure or
/// split lines need them.
#[must_use]
pub fn strip(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    scan(s.as_bytes(), |t| match t {
        Token::Print(c) => out.push(c),
        Token::Control(b) => out.push(char::from(b)),
        Token::Csi(_) | Token::Esc { .. } | Token::Osc(_) | Token::Opaque => {}
    });
    out
}

#[cfg(test)]
#[path = "escape_tests.rs"]
mod tests;
