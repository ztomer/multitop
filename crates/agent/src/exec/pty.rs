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
//! displays, and lets `sudo` open `/dev/tty` to prompt. stderr is a second pty,
//! which makes `isatty(2)` true and carries window dimensions for tools like
//! Docker Compose, Docker CLI, `BuildKit`, git and curl that inspect stderr to decide
//! whether to render interactive progress animations -- while keeping stderr
//! separable all the way to the panel.

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
fn named(call: &str, e: &io::Error) -> io::Error {
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
        // The fallbacks, by name rather than as bare numbers: this is the third
        // place the agent's default window was written, beside `Args::default`
        // and `serve`'s argument fallback, and the three are the same policy.
        ws_row: if rows == 0 {
            crate::consts::DEFAULT_PTY_ROWS
        } else {
            rows
        },
        ws_col: if cols == 0 {
            crate::consts::DEFAULT_PTY_COLS
        } else {
            cols
        },
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
        return Err(named("openpty", &io::Error::last_os_error()));
    }

    let mut err_master: libc::c_int = -1;
    let mut err_slave: libc::c_int = -1;
    let mut ws_err = ws;
    let rc_err = unsafe {
        libc::openpty(
            &raw mut err_master,
            &raw mut err_slave,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &raw mut ws_err,
        )
    };
    if rc_err != 0 {
        let e = named("openpty (stderr)", &io::Error::last_os_error());
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
        let e = named("fork", &io::Error::last_os_error());
        // SAFETY: all four were opened above and are live.
        unsafe {
            libc::close(master);
            libc::close(slave);
            libc::close(err_master);
            libc::close(err_slave);
        }
        return Err(e);
    }

    if pid == 0 {
        // SAFETY: the child. Every call here is async-signal-safe, and the
        // process ends in `execvp` or `_exit` -- never by unwinding, which
        // would run this process's destructors on a copy of the parent's heap.
        unsafe {
            libc::close(master);
            libc::close(err_master);
            // New session, slave becomes the controlling terminal, and 0/1/2
            // are duped from it. This is what `isatty(1)` answers, and what
            // `sudo` finds when it opens `/dev/tty`.
            if libc::login_tty(slave) != 0 {
                libc::_exit(127);
            }
            // Then take stderr off the primary pty and onto its own pty slave.
            // This is what `isatty(2)` answers, allowing tools like Docker Compose,
            // Docker CLI, `BuildKit`, git, and curl to detect an interactive terminal
            // for their progress displays while keeping stderr separable.
            if libc::dup2(err_slave, 2) < 0 {
                libc::_exit(127);
            }
            if err_slave > 2 {
                libc::close(err_slave);
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
        libc::close(err_slave);
    }
    Ok(Child {
        master,
        errpipe: err_master,
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
        if let Ok(n) = usize::try_from(n) {
            return Ok(n);
        }
        let e = io::Error::last_os_error();
        match e.raw_os_error() {
            // EINTR: retry. The loop continues on its own, so no `continue`.
            Some(libc::EINTR) => {}
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
        // A positive count is the bytes written; zero or negative is an error.
        if let Some(n) = usize::try_from(n).ok().filter(|&n| n > 0) {
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
pub const fn decode_status(status: libc::c_int) -> Outcome {
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

/// Set by `SIGTERM` or `SIGHUP` to the agent: the client asked the run to stop.
static STOP: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

extern "C" fn note_stop(_sig: libc::c_int) {
    // Async-signal-safe: one atomic store, nothing else.
    STOP.store(true, std::sync::atomic::Ordering::SeqCst);
}

/// Turn `SIGTERM` and `SIGHUP` to the agent into a request to stop the run.
///
/// At their default disposition either signal ends the agent on the spot, and
/// its teardown -- [`terminate`], which ends the child's whole session -- never
/// runs: the shell and its command are orphaned to init. With this installed
/// the pump sees [`stop_requested`] within one poll and tears the run down the
/// way it does for a client that vanished (M.KILLDROP). A forked child gets the
/// default dispositions back at `exec`, so the command is unaffected.
pub fn install_stop_handlers() {
    for sig in [libc::SIGTERM, libc::SIGHUP] {
        // SAFETY: a zeroed `sigaction` with a valid handler and empty mask is a
        // well-formed argument; the handler only stores to an atomic.
        unsafe {
            let mut act: libc::sigaction = std::mem::zeroed();
            act.sa_sigaction = note_stop as extern "C" fn(libc::c_int) as libc::sighandler_t;
            libc::sigemptyset(&raw mut act.sa_mask);
            libc::sigaction(sig, &raw const act, std::ptr::null_mut());
        }
    }
}

/// Whether the agent has been asked to stop (see [`install_stop_handlers`]).
#[must_use]
pub fn stop_requested() -> bool {
    STOP.load(std::sync::atomic::Ordering::SeqCst)
}

/// How long a child's session has to end after the hangup before it is killed.
const TERMINATE_GRACE: std::time::Duration = std::time::Duration::from_secs(2);
/// How often the grace looks for the leader having ended.
const TERMINATE_POLL: std::time::Duration = std::time::Duration::from_millis(20);

/// End the child's whole session and reap it.
///
/// The pid this module forked is not the whole tree. It is `/bin/sh`, the
/// session leader; the `zsh -l -i` or `bash -i` it starts turns job control
/// on and moves itself into a process group of its own, and every foreground
/// job the shell then runs gets a group of ITS own, which the terminal's
/// foreground group follows. A signal to the pid, or to the pid's group,
/// leaves the shell running -- with its parent gone, as an orphan of init.
///
/// So every group in the session is signalled, found by asking each process
/// which session it is in ([`super::session::groups`]). The terminal's
/// foreground group is NOT a stand-in for the shell's: while a loop's `sleep`
/// runs, it is the `sleep`'s group, the shell is outside it, and the hangup
/// missed the shell (M.REAPFLAKE). On Linux `bash -i` then outlived its run in 3 of 5 probes,
/// and CI run 42549434587 found it as the one survivor of a killed test;
/// on macOS the same probe left nothing in 20 of 20, which is why it never
/// showed there. `exec_reap_test` pins it: red 6 of 6 on Linux before this.
///
/// `SIGHUP` first, because that is what a terminal going away sends and what
/// an interactive shell honours -- it IGNORES `SIGTERM` -- then `SIGTERM` for
/// anything that ignores a hangup, and `SIGKILL` to every group still standing
/// once the grace is over. A `SIGKILL` alone would skip the cleanup an upgrade
/// in progress is owed (`apt`'s and `dpkg`'s own locks). The session is read
/// again before the `SIGKILL`: a shell forks new groups as it runs.
#[must_use]
pub fn terminate(child: &mut Child) -> Outcome {
    let sid = child.pid;
    let mut groups = super::session::groups(sid);
    if child.master >= 0 {
        // SAFETY: `master` is a live descriptor this module opened. Kept beside
        // the scan for a host whose process table hides other pids.
        let fg = unsafe { libc::tcgetpgrp(child.master) };
        if fg > 0 && !groups.contains(&fg) {
            groups.push(fg);
        }
    }
    let signal_all = |groups: &[libc::pid_t], sig: libc::c_int| {
        for g in groups {
            // SAFETY: a negative pid names a process group; at worst it is gone
            // and `kill` says ESRCH, which is the outcome being asked for.
            unsafe { libc::kill(-g, sig) };
        }
    };
    let kill_rest = |mut groups: Vec<libc::pid_t>| {
        for g in super::session::groups(sid) {
            if !groups.contains(&g) {
                groups.push(g);
            }
        }
        signal_all(&groups, libc::SIGKILL);
    };
    signal_all(&groups, libc::SIGHUP);
    signal_all(&groups, libc::SIGTERM);
    child.close();
    let deadline = std::time::Instant::now() + TERMINATE_GRACE;
    while std::time::Instant::now() < deadline {
        if let Some(outcome) = try_wait(child.pid) {
            // The leader is gone; a group member may not be. Kill what is left.
            kill_rest(groups);
            return outcome;
        }
        std::thread::sleep(TERMINATE_POLL);
    }
    kill_rest(groups);
    wait(child.pid)
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
