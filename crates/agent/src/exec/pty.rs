//! Starting a child on a pty this process owns.
//!
//! `openpty` + `fork` rather than `forkpty`, for one reason: the child needs a
//! **separate** stderr. `forkpty` puts fds 0, 1 and 2 all on the slave, which is
//! what a terminal does and also what loses the distinction the panel needs --
//! the reason a run failed is nearly always on stderr, and colouring it as
//! ordinary output buries it in a thousand lines of `apt`.
//!
//! So: stdin and stdout are the pty, and the pty is the controlling terminal,
//! which is what makes `isatty(1)` true, keeps colour and `\r` progress
//! displays, and lets `sudo` open `/dev/tty` to prompt. stderr is a pipe, which
//! costs nothing -- no tool decides its output style from `isatty(2)` -- and
//! keeps the failure reason separable all the way to the panel.

use std::ffi::CString;
use std::io;
use std::os::fd::RawFd;

/// A running child, and the two descriptors its output arrives on.
#[derive(Debug)]
pub struct Child {
    /// The pty master. Carries the child's stdout, and takes its stdin.
    pub master: RawFd,
    /// The read end of the child's stderr.
    pub errpipe: RawFd,
    pub pid: libc::pid_t,
}

impl Child {
    /// Close both descriptors. Separate from `Drop` because the reader wants
    /// to close each one as it reaches EOF and keep polling the other.
    pub fn close(&mut self) {
        for fd in [&mut self.master, &mut self.errpipe] {
            if *fd >= 0 {
                // SAFETY: each fd was opened by this module and is closed once;
                // it is set to -1 immediately so a second call cannot close a
                // descriptor number that has since been reused.
                unsafe { libc::close(*fd) };
                *fd = -1;
            }
        }
    }
}

/// How the child ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Outcome {
    pub code: i32,
    pub signalled: bool,
}

/// Say which call failed, and say something legible when the OS does not.
///
/// A spawn failure was once reported to a test as `Unknown error: -6 (os error
/// -6)`. -6 is not an errno; some call had returned failure without setting one,
/// or had clobbered it. "could not start a shell: Unknown error: -6" tells an
/// operator nothing at all -- not even which of the three syscalls involved
/// went wrong -- so the name is attached here where it is still known.
fn named(call: &str, e: io::Error) -> io::Error {
    let detail = match e.raw_os_error() {
        Some(n) if n <= 0 => format!("{call} failed and set no error code (rc {n})"),
        _ => format!("{call}: {e}"),
    };
    io::Error::new(e.kind(), detail)
}

/// Start `argv` on a fresh pty of the given size.
///
/// `argv` is built before the fork on purpose. Between `fork` and `exec` only
/// async-signal-safe calls are legal, and allocating a `CString` is not one of
/// them -- a malloc lock held by another thread at the moment of the fork is
/// held forever in the child, which hangs before it ever reaches `exec`.
///
/// # Errors
///
/// Any of `openpty`, `pipe` or `fork` failing.
pub fn spawn(argv: &[CString], cols: u16, rows: u16) -> io::Result<Child> {
    let mut master: libc::c_int = -1;
    let mut slave: libc::c_int = -1;
    let ws = libc::winsize {
        ws_row: rows.max(1),
        ws_col: cols.max(1),
        ws_xpixel: 0,
        ws_ypixel: 0,
    };

    // SAFETY: both out-params are owned locals, `name` is null (we never need
    // the slave's path, and asking for it is the one racy part of this API),
    // and `winp` points at a live local for the duration of the call.
    //
    // `*mut` for the last two rather than `*const`: Apple declares them mutable
    // and Linux does not, and `*mut T` coerces to `*const T` while the reverse
    // does not. One spelling that compiles for both beats a `cfg` whose second
    // arm is only ever built by the cross-compile.
    let mut ws = ws;
    let rc = unsafe {
        libc::openpty(
            &raw mut master,
            &raw mut slave,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &raw mut ws,
        )
    };
    if rc != 0 {
        return Err(named("openpty", io::Error::last_os_error()));
    }

    let mut errfds: [libc::c_int; 2] = [-1, -1];
    // SAFETY: `pipe` writes exactly two ints into an array of two.
    if unsafe { libc::pipe(errfds.as_mut_ptr()) } != 0 {
        let e = named("pipe", io::Error::last_os_error());
        // SAFETY: both were opened above and are live.
        unsafe {
            libc::close(master);
            libc::close(slave);
        }
        return Err(e);
    }

    let mut ptrs: Vec<*const libc::c_char> = argv.iter().map(|a| a.as_ptr()).collect();
    ptrs.push(std::ptr::null());

    // SAFETY: `fork`. The child below reaches `execv` through async-signal-safe
    // calls only, which is the rule that matters here and the reason the exec
    // is `execv` and not `execvp`: a PATH search allocates, and a `malloc` lock
    // held by another thread at the instant of the fork is held forever in the
    // child, which then hangs before it ever reaches the shell. The agent is
    // single-threaded in production, but its own test harness is not, and a
    // hazard that only shows up under threads is one that will eventually show
    // up in production too.
    let pid = unsafe { libc::fork() };
    if pid < 0 {
        let e = named("fork", io::Error::last_os_error());
        // SAFETY: all four were opened above and are live.
        unsafe {
            libc::close(master);
            libc::close(slave);
            libc::close(errfds[0]);
            libc::close(errfds[1]);
        }
        return Err(e);
    }

    if pid == 0 {
        // SAFETY: the child. Every call here is async-signal-safe, and the
        // process ends in `execvp` or `_exit` -- never by unwinding, which
        // would run this process's destructors on a copy of the parent's heap.
        unsafe {
            libc::close(master);
            libc::close(errfds[0]);
            // New session, slave becomes the controlling terminal, and 0/1/2
            // are duped from it. This is what `isatty(1)` answers, and what
            // `sudo` finds when it opens `/dev/tty`.
            if libc::login_tty(slave) != 0 {
                libc::_exit(127);
            }
            // Then take stderr back off the pty and onto its own pipe.
            if libc::dup2(errfds[1], 2) < 0 {
                libc::_exit(127);
            }
            if errfds[1] > 2 {
                libc::close(errfds[1]);
            }
            // `execv`, not `execvp`: argv[0] is an absolute path, so there is
            // no PATH search and therefore no allocation between here and the
            // new image.
            libc::execv(ptrs[0], ptrs.as_ptr());
            // Only reachable when exec failed: no shell at all on this host.
            libc::_exit(127);
        }
    }

    // SAFETY: the parent keeps only the ends it reads and writes.
    unsafe {
        libc::close(slave);
        libc::close(errfds[1]);
    }
    Ok(Child {
        master,
        errpipe: errfds[0],
        pid,
    })
}

/// Read what is ready, or `Ok(0)` at end of stream.
///
/// A pty master reports `EIO` rather than end-of-file when the last descriptor
/// on the slave side closes -- which is exactly what the child exiting does.
/// Reported as an error it looks like the host went away mid-upgrade; it is the
/// ordinary way this stream ends, so it is translated here rather than at each
/// of the two call sites.
///
/// # Errors
///
/// Any read error that is not the end of the stream.
pub fn read_fd(fd: RawFd, buf: &mut [u8]) -> io::Result<usize> {
    loop {
        // SAFETY: `buf` is a live slice and `n` is its length, so the kernel
        // writes only within it.
        let n = unsafe { libc::read(fd, buf.as_mut_ptr().cast(), buf.len()) };
        if n >= 0 {
            #[allow(clippy::cast_sign_loss)]
            return Ok(n as usize);
        }
        let e = io::Error::last_os_error();
        match e.raw_os_error() {
            Some(libc::EINTR) => continue,
            Some(libc::EIO) => return Ok(0),
            _ => return Err(e),
        }
    }
}

/// Write every byte, retrying short writes.
///
/// # Errors
///
/// Any write error other than an interruption.
pub fn write_fd(fd: RawFd, mut buf: &[u8]) -> io::Result<()> {
    while !buf.is_empty() {
        // SAFETY: `buf` is a live slice and `n` is its length.
        let n = unsafe { libc::write(fd, buf.as_ptr().cast(), buf.len()) };
        if n > 0 {
            #[allow(clippy::cast_sign_loss)]
            let n = n as usize;
            buf = &buf[n..];
            continue;
        }
        let e = io::Error::last_os_error();
        if e.raw_os_error() == Some(libc::EINTR) {
            continue;
        }
        return Err(e);
    }
    Ok(())
}

/// Wait up to `timeout_ms` for either descriptor to become readable.
///
/// Returns `(master_ready, errpipe_ready)`. A descriptor already closed (`-1`)
/// is never reported ready.
#[must_use]
pub fn poll_both(master: RawFd, errpipe: RawFd, timeout_ms: i32) -> (bool, bool) {
    let mut fds = [
        libc::pollfd {
            fd: master,
            events: libc::POLLIN,
            revents: 0,
        },
        libc::pollfd {
            fd: errpipe,
            events: libc::POLLIN,
            revents: 0,
        },
    ];
    // SAFETY: a live array of exactly the length passed.
    let rc = unsafe { libc::poll(fds.as_mut_ptr(), 2, timeout_ms) };
    if rc <= 0 {
        return (false, false);
    }
    // POLLHUP and POLLERR are readable events here: they are how the reader
    // learns to go and collect the end of the stream. Treating only POLLIN as
    // ready is how a closed pipe becomes a poll that spins until the timeout,
    // every time, for the rest of the run.
    let ready = |p: &libc::pollfd| {
        p.fd >= 0 && p.revents & (libc::POLLIN | libc::POLLHUP | libc::POLLERR) != 0
    };
    (ready(&fds[0]), ready(&fds[1]))
}

/// Reap the child, blocking until it ends.
#[must_use]
pub fn wait(pid: libc::pid_t) -> Outcome {
    let mut status: libc::c_int = 0;
    loop {
        // SAFETY: `status` is a live local.
        let rc = unsafe { libc::waitpid(pid, &raw mut status, 0) };
        if rc < 0 && io::Error::last_os_error().raw_os_error() == Some(libc::EINTR) {
            continue;
        }
        if rc < 0 {
            // The child is unreachable. Say so as a distinct code rather than
            // reporting a success nobody observed.
            return Outcome {
                code: -1,
                signalled: false,
            };
        }
        return decode_status(status);
    }
}

/// Turn a `waitpid` status into an outcome.
///
/// Split out because the bit layout is the kind of thing that is written once,
/// read as obviously right, and wrong: a signalled child has to be reported as
/// signalled, not as "exited 0", or a killed upgrade is announced as a success.
#[must_use]
pub fn decode_status(status: libc::c_int) -> Outcome {
    if libc::WIFSIGNALED(status) {
        return Outcome {
            code: 128 + libc::WTERMSIG(status),
            signalled: true,
        };
    }
    Outcome {
        code: libc::WEXITSTATUS(status),
        signalled: false,
    }
}

/// Ask whether the child has ended, without blocking.
#[must_use]
pub fn try_wait(pid: libc::pid_t) -> Option<Outcome> {
    let mut status: libc::c_int = 0;
    // SAFETY: `status` is a live local; `WNOHANG` makes this non-blocking.
    let rc = unsafe { libc::waitpid(pid, &raw mut status, libc::WNOHANG) };
    if rc == pid {
        return Some(decode_status(status));
    }
    None
}
