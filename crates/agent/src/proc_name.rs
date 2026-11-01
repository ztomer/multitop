//! A process name, held inline: no heap allocation, ever.
//!
//! The scan reads every process on the host, every tick. Giving each name its
//! own `String` made a warm tick allocate in proportion to the process table
//! -- ~900 names, three copies a tick, on a busy server -- and the static
//! agent's allocator (musl's) kept the freed slots instead of reusing them:
//! 13.6 MB and growing on .33 on 2026-11-01, where the same code on glibc sat
//! at 520 kB (roadmap M1). A name has a ceiling set by the kernel, so it can
//! live in a fixed array instead, and then a tick does no heap work per process
//! at all, whatever the allocator.
//!
//! The ceiling: Linux truncates `comm` to 15 bytes (`TASK_COMM_LEN` is 16 with
//! the NUL), and macOS's `proc_name` returns at most `2 * MAXCOMLEN` = 32.
//! Anything longer -- only a name decoded off the wire from a sender this
//! crate did not write could be -- is cut at the last character boundary that
//! fits, which is what the kernels themselves do.

use crate::consts::PROC_NAME_CAP;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct ProcName {
    len: u8,
    bytes: [u8; PROC_NAME_CAP],
}

impl ProcName {
    /// `s`, cut to the longest prefix of whole characters that fits.
    #[must_use]
    pub fn new(s: &str) -> Self {
        let mut end = s.len().min(PROC_NAME_CAP);
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        let mut bytes = [0u8; PROC_NAME_CAP];
        bytes[..end].copy_from_slice(&s.as_bytes()[..end]);
        Self {
            // `end <= PROC_NAME_CAP`, which `consts` holds below `u8::MAX`.
            len: u8::try_from(end).unwrap_or(u8::MAX),
            bytes,
        }
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        // Built only from a `&str` cut at a character boundary, so always UTF-8.
        std::str::from_utf8(&self.bytes[..usize::from(self.len)]).unwrap_or_default()
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }
}

impl Default for ProcName {
    fn default() -> Self {
        Self {
            len: 0,
            bytes: [0; PROC_NAME_CAP],
        }
    }
}

/// By the name, as a `str` orders: the list the filter searches is sorted so
/// an unchanged host sends the same bytes every tick.
impl Ord for ProcName {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.as_str().cmp(other.as_str())
    }
}

impl PartialOrd for ProcName {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl std::ops::Deref for ProcName {
    type Target = str;
    fn deref(&self) -> &str {
        self.as_str()
    }
}

impl AsRef<str> for ProcName {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl From<&str> for ProcName {
    fn from(s: &str) -> Self {
        Self::new(s)
    }
}

impl std::fmt::Debug for ProcName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(self.as_str(), f)
    }
}

impl std::fmt::Display for ProcName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl PartialEq<str> for ProcName {
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}

impl PartialEq<&str> for ProcName {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_that_fits_is_kept_whole() {
        assert_eq!(ProcName::new("containerd-shim"), "containerd-shim");
        multitop_testassert::assert_empty!(ProcName::new(""));
    }

    #[test]
    fn a_long_name_is_cut_at_a_character_boundary() {
        let long = "x".repeat(PROC_NAME_CAP + 10);
        assert_eq!(ProcName::new(&long).len(), PROC_NAME_CAP);
        // A two-byte character straddling the cap is dropped whole, not split.
        let straddle = format!("{}é", "x".repeat(PROC_NAME_CAP - 1));
        assert_eq!(
            ProcName::new(&straddle).as_str(),
            "x".repeat(PROC_NAME_CAP - 1)
        );
    }
}
