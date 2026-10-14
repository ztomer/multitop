//! The `--serve` token: how it is made, and how it is compared.
//!
//! This is the only thing between the network and every host's metrics once
//! `multitop --serve 0.0.0.0:8080` is running — the bind the phone companion needs.
//! On loopback it does not matter. On a LAN it is the whole of the authentication.
//!
//! **What was wrong here, 2026-10-14 (ROADMAP P0.1).** The token was generated
//! with `DefaultHasher` over the process id and a nanosecond timestamp. That is
//! `SipHash` with FIXED keys, so the token was a deterministic function of two
//! values an attacker can bound — the pid is small and the start time is roughly
//! known — and it was 64 bits, half the floor for a credential. The generator's own
//! comment claimed a rule it did not implement ("only when binding to loopback ...
//! For now, generate if not supplied").
//!
//! Three defects, one place, fixed together:
//!
//! 1. **Source.** At least 128 bits from the OS CSPRNG. Never derived from time
//!    or pid: both are observable, and a credential built from observable inputs
//!    is a guessable credential however well it is hashed.
//! 2. **Comparison.** `got == expected` short-circuits on the first differing
//!    byte, so a caller who can time responses learns the token a byte at a time.
//!    [`matches`] compares in constant time.
//! 3. **State.** The token was `Option<String>` and "no token" meant "no
//!    authentication" — see [`crate::server::check_auth`], which no longer takes
//!    one.

use std::fmt::Write as _;

use subtle::ConstantTimeEq;

/// Bytes of entropy per generated token: 32 bytes = 256 bits, twice the 128-bit
/// floor. Hex-encoded to 64 characters.
const TOKEN_BYTES: usize = 32;

/// A fresh token, hex-encoded, from the OS CSPRNG.
///
/// # Errors
///
/// When the OS CSPRNG fails, and only then. There is no fallback, and that is
/// deliberate: the alternatives are a weaker source or a token derived from
/// something observable, and a `--serve` on a LAN with a guessable token is worse
/// than a refusal to start. The error is returned rather than panicked on, so the
/// one caller can print something useful and exit non-zero — a panic in a CLI that
/// has just bound a port is a worse report than a sentence.
pub fn generate() -> Result<String, std::io::Error> {
    let mut bytes = [0u8; TOKEN_BYTES];
    getrandom::fill(&mut bytes)?;
    // `write!` rather than `format!` per byte: this runs once per server start, and
    // the map-and-collect spelling is both an allocation per byte and a clippy error
    // for building a string from an iterator.
    let mut out = String::with_capacity(TOKEN_BYTES * 2);
    for byte in bytes {
        // Writing into a String cannot fail; `write!` returns a Result either way.
        let _ = write!(out, "{byte:02x}");
    }
    Ok(out)
}

/// True when `got` is `expected`, in time that does not depend on where they first
/// differ.
///
/// Length is compared first and returns early, which leaks only the length — and
/// both values here are fixed-length hex, so that is not a secret. Everything beyond
/// it runs to completion regardless of the contents.
#[must_use]
pub fn matches(expected: &str, got: &str) -> bool {
    if expected.len() != got.len() {
        return false;
    }
    expected.as_bytes().ct_eq(got.as_bytes()).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `generate` is infallible on every platform this ships to, so the tests unwrap
    /// where production propagates. `unwrap_used` is denied workspace-wide; the deny
    /// is about a library silently guessing, and a test that cannot get randomness is
    /// not something to keep running past.
    fn token() -> String {
        match generate() {
            Ok(t) => t,
            Err(e) => panic!("the OS CSPRNG failed in a test: {e}"),
        }
    }

    #[test]
    fn a_generated_token_carries_at_least_128_bits() {
        // RED before the fix: the old generator returned `{:016x}` of a u64 — 16
        // hex characters, 64 bits, half the floor.
        let token = token();
        assert!(
            token.len() >= 32,
            "token is {} hex chars ({token:?}) — {} bits, below the 128-bit floor",
            token.len(),
            token.len() * 4
        );
    }

    #[test]
    fn a_generated_token_is_hex() {
        assert!(token().chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn a_generated_token_is_the_length_it_promises() {
        // The hex arm is a per-byte loop with a discarded Result. If `write!` ever
        // stopped advancing, this is what notices — a short token is the failure that
        // the entropy floor test would otherwise report as a weaker floor.
        assert_eq!(token().len(), TOKEN_BYTES * 2);
    }

    #[test]
    fn two_generated_tokens_differ() {
        // Not a claim about the CSPRNG's quality — a claim that it was consulted at
        // all. The old generator was a function of pid and a timestamp, so two calls
        // collided only if they shared a pid AND landed in the same nanosecond; that
        // is "unlikely", not "impossible".
        assert_ne!(token(), token());
    }

    #[test]
    fn the_right_token_matches() {
        assert!(matches("deadbeef", "deadbeef"));
    }

    #[test]
    fn a_wrong_token_does_not_match() {
        assert!(!matches("deadbeef", "deadbeee"));
        assert!(!matches("deadbeef", "beefdead"));
    }

    #[test]
    fn a_different_length_never_matches() {
        assert!(!matches("deadbeef", "deadbeeff"));
        assert!(!matches("deadbeef", "dead"));
        assert!(!matches("", "x"));
    }

    #[test]
    fn an_empty_token_does_not_match_a_real_one() {
        // The shape a bare `Authorization: Bearer ` produces: the prefix strips and
        // nothing is left.
        assert!(!matches("deadbeef", ""));
    }
}
