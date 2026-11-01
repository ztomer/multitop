//! A warm tick does no heap work per process, and so the agent's resident
//! memory does not grow with the number of ticks.
//!
//! The agent samples every two seconds for as long as a client is connected --
//! hours, on a server. On 2026-11-01 the shipped agent was found at 13.6 MB on
//! .33 and growing (roadmap M1). Nothing was leaked in Rust's sense: every
//! tick freed what it allocated. But every tick allocated a fresh `String` per
//! process -- for the scan, for the name list the filter searches, and again
//! for a clone of that list to encode it -- and then a fresh packet buffer that
//! doubled its way from 512 B to 16 KiB to hold the frame. musl's allocator,
//! which is what the static agent ships on, kept the freed memory instead of
//! reusing it. The same code on glibc sat at 520 kB.
//!
//! So the invariant is not "frees what it allocates", which held all along. It
//! is "a warm tick allocates nothing in proportion to the host": names live
//! inline in a fixed-size `ProcName`, and the process table, the name list and
//! the packet are buffers the loop keeps, which grow only when the host runs
//! more -- or more distinctly named -- processes than they have held before.
//! That is measured here the way that holds on every allocator: a counting
//! global allocator around one tick, at two populations of distinctly named
//! processes. On Linux the outcome the owner saw is measured too -- `RssAnon`
//! over hundreds of ticks of a churning table.
//!
//! Its own test binary, and one test: the allocator counts the whole process.

#![cfg(test)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

use multitop_agent::monitor::Monitor;
use multitop_agent::palette_view::PaletteView;
use multitop_agent::{emit_monitor, parse_args, Args, FrameBuf, SortBy};

struct Counting;

static ALLOCS: AtomicUsize = AtomicUsize::new(0);
static BYTES: AtomicUsize = AtomicUsize::new(0);

// SAFETY: every method forwards to `System` with the caller's own arguments;
// the counters are the only addition and they never touch the memory.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        BYTES.fetch_add(layout.size(), Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        BYTES.fetch_add(new_size.saturating_sub(layout.size()), Ordering::Relaxed);
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

/// Name stems of the lengths a server's process table holds, from the census of
/// the .33 agent's heap -- the kernel's 15-byte cap included.
const NAMES: &[&str] = &[
    "containerd-shim",
    "s6-supervise",
    "gunicorn",
    "docker-proxy",
    "postgres",
    "MainThread",
    "python3",
    "bash",
    "nginx",
    "celery",
    "dotnet",
    "forkserver",
    "gluetun-entrypo",
    "sh",
    "redis-server",
    "node",
    "java",
    "kworker-u8",
    "systemd-journal",
    "cron",
];

/// How many distinct names the fake server runs. Distinct matters: the list the
/// filter searches is deduplicated, so twenty names repeated would never fill
/// a frame the way .33's hundreds of names do -- which is the frame size at
/// which a rebuilt packet buffer made musl grow.
const DISTINCT_NAMES: usize = 600;

/// The kernel's cap on a process name.
const COMM_LEN: usize = 15;

/// The children's own deadline, so a run that is killed outright cannot leave
/// them behind for longer than this.
const CHILD_LIFETIME_SECS: &str = "300";

/// A process table that churns. Its children are killed and reaped when it is
/// dropped, pass or fail.
struct Churn {
    links: Vec<PathBuf>,
    children: std::collections::VecDeque<Child>,
    next: usize,
    _dir: tempfile::TempDir,
}

impl Churn {
    fn new() -> Self {
        let sleep = ["/bin/sleep", "/usr/bin/sleep"]
            .into_iter()
            .map(PathBuf::from)
            .find(|p| p.exists())
            .expect("a sleep binary");
        let dir = tempfile::tempdir().expect("tempdir");
        // The process name is the basename of what was executed, so a symlink
        // per name gives every child a name of its own.
        let links = (0..DISTINCT_NAMES)
            .map(|i| {
                let stem = NAMES[i % NAMES.len()];
                let digits = i.to_string();
                let keep = stem.len().min(COMM_LEN - digits.len());
                let link = dir.path().join(format!("{}{digits}", &stem[..keep]));
                std::os::unix::fs::symlink(&sleep, &link).expect("symlink");
                link
            })
            .collect();
        Self {
            links,
            children: std::collections::VecDeque::new(),
            next: 0,
            _dir: dir,
        }
    }

    fn spawn(&mut self) {
        let link = &self.links[self.next % self.links.len()];
        self.next += 1;
        let child = Command::new(link)
            .arg(CHILD_LIFETIME_SECS)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn a churn child");
        self.children.push_back(child);
    }

    fn grow_to(&mut self, population: usize) {
        while self.children.len() < population {
            self.spawn();
        }
    }

    /// Replace `n` of the oldest processes with new ones.
    fn turn(&mut self, n: usize) {
        for _ in 0..n {
            if let Some(mut old) = self.children.pop_front() {
                let _ = old.kill();
                let _ = old.wait();
            }
            self.spawn();
        }
    }
}

impl Drop for Churn {
    fn drop(&mut self) {
        for c in &mut self.children {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

/// What the agent does per tick, exactly: sample, then encode the frame for
/// the pipe.
fn tick(m: &mut Monitor, args: &Args, view: &PaletteView, buf: &mut FrameBuf) {
    let snap = m.tick(2.0, 120, 50, SortBy::Cpu);
    emit_monitor(snap, args, false, view, buf, &mut std::io::sink()).expect("sink");
}

/// Allocations and bytes one warm tick makes, the median over a few ticks so a
/// one-off (a buffer meeting a new high-water mark) does not count.
fn per_tick(m: &mut Monitor, churn: &mut Churn, args: &Args, view: &PaletteView) -> (usize, usize) {
    let mut buf = FrameBuf::default();
    for _ in 0..10 {
        churn.turn(2);
        tick(m, args, view, &mut buf);
    }
    let mut samples: Vec<(usize, usize)> = (0..9)
        .map(|_| {
            churn.turn(2);
            let (a, b) = (
                ALLOCS.load(Ordering::Relaxed),
                BYTES.load(Ordering::Relaxed),
            );
            tick(m, args, view, &mut buf);
            (
                ALLOCS.load(Ordering::Relaxed) - a,
                BYTES.load(Ordering::Relaxed) - b,
            )
        })
        .collect();
    samples.sort_unstable();
    samples[samples.len() / 2]
}

#[cfg(target_os = "linux")]
fn rss_anon_kb() -> usize {
    let status = std::fs::read_to_string("/proc/self/status").expect("status");
    status
        .lines()
        .find_map(|l| l.strip_prefix("RssAnon:"))
        .and_then(|v| v.split_whitespace().next())
        .and_then(|v| v.parse().ok())
        .expect("RssAnon in /proc/self/status")
}

/// The outcome the owner saw: resident memory over many ticks of a server-sized
/// table that churns. Before the fix this grew ~7 kB a tick on musl at 900
/// processes (1,224 kB to 5,464 kB over 600 ticks).
#[cfg(target_os = "linux")]
fn assert_resident_memory_is_flat(
    m: &mut Monitor,
    churn: &mut Churn,
    args: &Args,
    view: &PaletteView,
) {
    churn.grow_to(900);
    let mut buf = FrameBuf::default();
    let mut run = |ticks: usize, churn: &mut Churn| {
        for _ in 0..ticks {
            churn.turn(5);
            tick(m, args, view, &mut buf);
        }
    };
    run(100, churn);
    let warm = rss_anon_kb();
    run(600, churn);
    let late = rss_anon_kb();
    // The working set is a few hundred kB; the failure kept megabytes.
    let slack_kb = 512;
    assert!(
        late <= warm + slack_kb,
        "RssAnon grew {} kB over 600 ticks (warm {warm} kB, late {late} kB)",
        late.saturating_sub(warm)
    );
}

#[cfg(not(target_os = "linux"))]
const fn assert_resident_memory_is_flat(_: &mut Monitor, _: &mut Churn, _: &Args, _: &PaletteView) {
}

#[test]
fn a_warm_tick_allocates_nothing_per_process_and_memory_stays_flat() {
    let args = parse_args(["monitor", "rss-bound"].map(String::from));
    let view = PaletteView::for_theme(&multitop_agent::color::PLAIN);
    let mut churn = Churn::new();
    let mut m = Monitor::new("rss-bound".into());

    // Both populations fill the panel, so the rows it draws -- a `String`
    // each, and bounded by the panel, not the host -- are the same count.
    churn.grow_to(200);
    let (small_allocs, small_bytes) = per_tick(&mut m, &mut churn, &args, &view);
    churn.grow_to(600);
    let (large_allocs, large_bytes) = per_tick(&mut m, &mut churn, &args, &view);

    // A warm tick allocates nothing at all, on Linux where the agent ships and
    // on the Mac where the local panel's agent runs: every name is inline,
    // every list a buffer the loop keeps, every pseudofile streamed through
    // the stack, and the Mach and IOKit samplers fill the monitor's buffers
    // (roadmap M2). Zero, at both sizes, so a single new allocation anywhere
    // in the tick is red, not only one that grows with the host.
    assert_eq!(
        (small_allocs, small_bytes, large_allocs, large_bytes),
        (0, 0, 0, 0),
        "a warm tick allocated (allocations, bytes): {small_allocs}, {small_bytes} at 200 \
         processes; {large_allocs}, {large_bytes} at 600"
    );

    assert_resident_memory_is_flat(&mut m, &mut churn, &args, &view);
}
