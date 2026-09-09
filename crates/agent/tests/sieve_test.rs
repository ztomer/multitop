//! Regression tests for the sieve's carriage-return handling.
//!
//! Markers arrive on lines a progress bar already wrote to
//! (`working...\r__multitop_sudo_failed__\n`), and reads split anywhere —
//! including mid-marker. These drive [`Sieve`] directly so they do not depend
//! on pty chunk timing the way the end-to-end exec tests do.
use multitop_agent::exec::sieve::{Piece, Sieve};
use multitop_agent::exec::MarkerKind;

fn marks(pieces: &[Piece]) -> Vec<MarkerKind> {
    pieces
        .iter()
        .filter_map(|p| match p {
            Piece::Mark(k) => Some(*k),
            Piece::Out(_) => None,
        })
        .collect()
}

fn feed_all(sieve: &mut Sieve, chunks: &[&[u8]]) -> Vec<Piece> {
    let mut out = Vec::new();
    for c in chunks {
        out.extend(sieve.feed(c));
    }
    out.extend(sieve.finish());
    out
}

/// The defect: a marker split across two reads after a `\r` prefix was
/// flushed as output and lost, because the hold-back length budget measured
/// the whole partial (progress text included) instead of the post-`\r` state.
#[test]
fn chunked_marker_after_carriage_return_is_still_a_marker() {
    let mut s = Sieve::new();
    let out = feed_all(&mut s, &[b"working...\r__multitop_sudo", b"_failed__\n"]);
    assert_eq!(marks(&out), vec![MarkerKind::SudoFailed]);
}

/// Control: the same split with no progress-bar prefix always worked.
#[test]
fn chunked_marker_without_prefix_is_still_a_marker() {
    let mut s = Sieve::new();
    let out = feed_all(&mut s, &[b"__multitop_sudo", b"_failed__\n"]);
    assert_eq!(marks(&out), vec![MarkerKind::SudoFailed]);
}

/// Control: an unsplit marker after `\r` always worked.
#[test]
fn whole_marker_after_carriage_return_is_still_a_marker() {
    let mut s = Sieve::new();
    let out = feed_all(&mut s, &[b"working...\r__multitop_sudo_failed__\n"]);
    assert_eq!(marks(&out), vec![MarkerKind::SudoFailed]);
}

/// The length budget still holds: a long markerless line is released, not
/// buffered forever.
#[test]
fn long_markerless_output_is_not_buffered() {
    let mut s = Sieve::new();
    let line = vec![b'x'; 4096];
    let out = feed_all(&mut s, &[&line]);
    assert!(marks(&out).is_empty());
    let text: Vec<u8> = out
        .iter()
        .flat_map(|p| match p {
            Piece::Out(b) => b.clone(),
            Piece::Mark(_) => Vec::new(),
        })
        .collect();
    assert_eq!(text, line);
}
