//! The wall clock, in one place: the epoch seconds the modules read.
//!
//! One implementation, so two modules cannot disagree about what "now" is.
//! The kept answers carry the moment they were taken, and the tool list is
//! kept under a window the server gave: both are read against this clock,
//! and a second copy of it would be a second answer to "what time is it".

/// Epoch seconds now.
#[must_use]
pub fn now_epoch() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}
