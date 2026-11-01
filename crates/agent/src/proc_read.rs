//! Reading pseudofiles: whole, and without the heap.
//!
//! Kernel pseudofiles report `st_size = 0`, so nothing can be pre-sized from
//! the file, and a `read` may return less than the file holds. Two shapes
//! follow, chosen by who decides the file's size:
//!
//! - **The host decides** (`/proc/stat`, `/proc/meminfo`, `/proc/net/dev`):
//!   `for_each_line` streams the file through one fixed stack buffer, a line
//!   at a time, to the end. A fixed buffer read once was the old shape, and it
//!   truncated: `/proc/net/dev` was 5,991 bytes on .33 and the agent counted
//!   the interfaces in its first 2 KiB (roadmap M2).
//! - **The kernel bounds it** (`/proc/<pid>/stat`, `/proc/<pid>/comm`):
//!   `read_proc_bytes` fills a caller-sized array, which is sized above the
//!   kernel's ceiling.

use std::fs;
use std::io::Read;
use std::path::Path;

use crate::consts::PROC_LINE_BUF;

/// Feed every line of the file at `path` to `line`, without its newline, in
/// order, through a fixed stack buffer: no heap allocation, and no file size
/// the host can outgrow.
///
/// A line longer than the buffer cannot be held, so it is passed over whole
/// and the lines after it are read as usual. None of the rows a sampler reads
/// comes near that length (a `cpu` row, an interface row and a `meminfo` row
/// are under 200 bytes); the rows that do are ones nothing samples, like
/// `/proc/stat`'s `intr`, which grows with the IRQ count. A line that is not
/// UTF-8 is passed over too.
///
/// Returns whether the file was read to its end and held something. A file
/// that failed part-way is `false` even though some lines were fed: a total
/// over part of the file is a wrong number, and the caller discards it.
pub fn for_each_line<P: AsRef<Path>>(path: P, mut line: impl FnMut(&str)) -> bool {
    let Ok(mut file) = fs::File::open(path) else {
        return false;
    };
    let mut buf = [0u8; PROC_LINE_BUF];
    let mut emit = |bytes: &[u8]| {
        if let Ok(s) = std::str::from_utf8(bytes) {
            line(s);
        }
    };
    // `filled` bytes at the front of `buf` are read and not yet fed; while
    // `skipping`, they belong to a line too long to hold.
    let (mut filled, mut skipping, mut any) = (0, false, false);
    loop {
        let Ok(n) = file.read(&mut buf[filled..]) else {
            return false;
        };
        if n == 0 {
            if filled > 0 && !skipping {
                emit(&buf[..filled]);
            }
            return any;
        }
        any = true;
        filled += n;
        let mut start = 0;
        while let Some(pos) = buf[start..filled].iter().position(|&b| b == b'\n') {
            if skipping {
                skipping = false;
            } else {
                emit(&buf[start..start + pos]);
            }
            start += pos + 1;
        }
        if start == 0 && filled == buf.len() {
            // One line fills the buffer: drop what is held of it, and the
            // rest of it as it arrives, up to its newline.
            skipping = true;
            filled = 0;
        } else {
            buf.copy_within(start..filled, 0);
            filled -= start;
        }
    }
}

/// Format `args` into `buf`, for a path that must not cost a heap allocation;
/// `None` when it does not fit.
pub fn fmt_into<'a>(buf: &'a mut [u8], args: std::fmt::Arguments<'_>) -> Option<&'a str> {
    use std::io::Write;
    let mut cur = std::io::Cursor::new(&mut buf[..]);
    cur.write_fmt(args).ok()?;
    let n = usize::try_from(cur.position()).ok()?;
    std::str::from_utf8(&buf[..n]).ok()
}

/// Read a file the kernel bounds into a caller-owned stack buffer, with zero
/// heap allocation, filling it to the end of the file or of the buffer.
///
/// For a file the host can make any size, use `for_each_line`: this stops
/// where the buffer does.
pub fn read_proc_bytes<P: AsRef<Path>, const N: usize>(path: P, out: &mut [u8; N]) -> usize {
    let Ok(mut file) = fs::File::open(path) else {
        return 0;
    };
    let mut filled = 0;
    while filled < N {
        match file.read(&mut out[filled..]) {
            Ok(0) | Err(_) => break,
            Ok(n) => filled += n,
        }
    }
    filled
}

/// Read a pseudofile into a caller-supplied String without allocating new memory buffers.
pub fn read_proc_into<P: AsRef<Path>>(path: P, out: &mut String) -> bool {
    let Ok(mut file) = fs::File::open(path) else {
        return false;
    };
    let mut buf = [0u8; crate::consts::PROC_CHUNK_BUF];
    loop {
        match file.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                if let Ok(s) = std::str::from_utf8(&buf[..n]) {
                    out.push_str(s);
                } else {
                    out.push_str(&String::from_utf8_lossy(&buf[..n]));
                }
            }
            Err(_) => return false,
        }
    }
    !out.is_empty()
}

pub fn read_proc<P: AsRef<Path>>(path: P) -> String {
    let mut out = String::with_capacity(crate::consts::PROC_LINE_CAPACITY);
    read_proc_into(path, &mut out);
    out
}
