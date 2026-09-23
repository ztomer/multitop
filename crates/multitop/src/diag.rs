//! Out-of-band diagnostics: what to do when the app stops answering.
//!
//! When the UI freezes there is nothing left to look at -- no key is served and
//! no pane redraws -- so the view that owns the terminal cannot report on its
//! own death. This module is the instrument for that moment: SIGUSR1 and SIGUSR2
//! are answered by a thread that does not touch the event loop, writing a
//! timestamped dump of the loop's phase, its counters, and (when available) a
//! snapshot of the app state to `$TMPDIR/multitop-diag-<pid>-<n>-*.txt`.
//!
//! Two tiers:
//! - The **signal tier** always lands, even when the loop is wedged: it is
//!   written by the signal thread from atomics alone, plus a `try_lock` read of
//!   the latest snapshot. Its own phase + counter lines say whether the loop
//!   stopped *between* polls (idle) or *inside* one (HandlingKey/Applying/
//!   Drawing/Resizing) -- which is the first bisection of any freeze.
//! - The **state tier** lands when the loop is healthy enough to answer: the
//!   signal sets a flag the loop notices on its next poll; the loop, which is
//!   the only thing that may touch `App`, snapshots it and writes the richer
//!   file itself. A wedge is precisely the case where the state tier never
//!   appears while the signal tier does.
//!
//! The snapshot never carries secrets. `PanelDigest` exists so the durable
//! view of a panel cannot include `sudo_password` or the vault; the compiler
//! enforces that the write path reads only the digest.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Mutex, Once};
use std::time::Instant;

use crate::app::App;
use crate::run::Tasks;

/// What the event-loop thread is doing right now. Written by the loop before
/// each window of work, so a wedge shows up as the window it died in, and
/// cleared to [`Phase::Idle`] once the poll is over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Setup,
    Idle,
    Drawing,
    HandlingKey,
    Applying,
    Resizing,
}

impl Phase {
    const fn code(self) -> u8 {
        match self {
            Self::Setup => 0,
            Self::Idle => 1,
            Self::Drawing => 2,
            Self::HandlingKey => 3,
            Self::Applying => 4,
            // magic-ok: a stable cross-process ordinal pinned by the round-trip test
            Self::Resizing => 5,
        }
    }

    const fn from_code(code: u8) -> Self {
        match code {
            0 => Self::Setup,
            1 => Self::Idle,
            2 => Self::Drawing,
            3 => Self::HandlingKey,
            4 => Self::Applying,
            _ => Self::Resizing,
        }
    }

    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Setup => "Setup",
            Self::Idle => "Idle",
            Self::Drawing => "Drawing",
            Self::HandlingKey => "HandlingKey",
            Self::Applying => "Applying",
            Self::Resizing => "Resizing",
        }
    }
}

/// What a task bag looks like from the diagnostic's seat: who is still running
/// and who has finished without anyone answering.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Liveness {
    pub monitors_alive: usize,
    pub upgrades_alive: usize,
    pub upgrades_done: usize,
}

/// The only per-panel view a dump may contain. Deliberately free of every
/// field that could hold a credential.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PanelDigest {
    pub host: String,
    pub mode: String,
    pub state: String,
    pub gen: u64,
    pub upgrade_gen: u64,
    pub view_len: usize,
    pub ring_len: usize,
    pub scroll: usize,
}

/// The quit sequence as the loop saw it: `armed` by the first `q`,
/// `requested` once confirmed.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct QuitFlags {
    pub armed: bool,
    pub requested: bool,
}

/// A point-in-time look at the app, safe to copy out from under the loop.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub mode: String,
    pub filter: String,
    pub selected: usize,
    pub panels: Vec<PanelDigest>,
    pub in_flight: bool,
    pub quit: QuitFlags,
    pub vault_unlocked: bool,
    pub last_update: Option<u64>,
    pub upgrade_started_at: Option<u64>,
    pub tasks: Liveness,
    pub active_confirm: Option<String>,
}

/// Shared diagnostic state. Laterally safe by construction: every field is an
/// atomic or a mutex, never a borrow of the loop's data.
pub struct Diag {
    phase: AtomicU8,
    iter: AtomicU64,
    keys: AtomicU64,
    applied: AtomicU64,
    drained: AtomicU64,
    requested: AtomicBool,
    requested_sig: AtomicU8,
    requested_seq: AtomicU64,
    seq: AtomicU64,
    signals_seen: AtomicU64,
    ready: AtomicBool,
    snapshot: Mutex<Option<Snapshot>>,
    out_dir: PathBuf,
    start: Instant,
}

impl Diag {
    #[must_use]
    pub fn new(out_dir: PathBuf) -> Arc<Self> {
        Arc::new(Self {
            phase: AtomicU8::new(Phase::Setup.code()),
            iter: AtomicU64::new(0),
            keys: AtomicU64::new(0),
            applied: AtomicU64::new(0),
            drained: AtomicU64::new(0),
            requested: AtomicBool::new(false),
            requested_sig: AtomicU8::new(0),
            requested_seq: AtomicU64::new(0),
            seq: AtomicU64::new(0),
            signals_seen: AtomicU64::new(0),
            ready: AtomicBool::new(false),
            snapshot: Mutex::new(None),
            out_dir,
            start: Instant::now(),
        })
    }

    #[must_use]
    pub fn default_dir() -> PathBuf {
        std::env::temp_dir()
    }

    pub fn set_phase(&self, phase: Phase) {
        self.phase.store(phase.code(), Ordering::Relaxed);
    }

    pub fn bump_iter(&self) {
        self.iter.fetch_add(1, Ordering::Relaxed);
    }

    pub fn bump_key(&self) {
        self.keys.fetch_add(1, Ordering::Relaxed);
    }

    pub fn bump_applied(&self) {
        self.applied.fetch_add(1, Ordering::Relaxed);
    }

    pub fn bump_drained(&self, n: usize) {
        self.drained.fetch_add(n as u64, Ordering::Relaxed);
    }

    fn next_seq(&self) -> u64 {
        self.seq.fetch_add(1, Ordering::Relaxed)
    }

    /// A signal arrived. The counter lets a test wait for the whole
    /// round-trip without racing the filesystem.
    pub fn note_signal(&self, sig: i32) {
        self.signals_seen.fetch_add(1, Ordering::Relaxed);
        self.requested.store(true, Ordering::Relaxed);
        self.requested_sig
            .store(u8::try_from(sig).unwrap_or(0), Ordering::Relaxed);
        self.requested_seq.store(self.next_seq(), Ordering::Relaxed);
    }

    /// The loop side of the two-tier handshake: returns one pending request,
    /// or `None` if nothing was asked for. Returning means the request is
    /// consumed, so a second poll cannot write the dump twice.
    #[must_use]
    pub fn take_request(&self) -> Option<(u64, &'static str)> {
        if !self.requested.swap(false, Ordering::Relaxed) {
            return None;
        }
        let sig = self.requested_sig.load(Ordering::Relaxed);
        Some((
            self.requested_seq.load(Ordering::Relaxed),
            Self::sig_name(i32::from(sig)),
        ))
    }

    const fn sig_name(sig: i32) -> &'static str {
        if sig == signal_hook::consts::SIGUSR2 {
            "USR2"
        } else {
            "USR1"
        }
    }

    pub fn store_snapshot(&self, snap: Snapshot) {
        let mut guard = match self.snapshot.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        *guard = Some(snap);
    }

    fn latest_snapshot(&self) -> Option<Snapshot> {
        match self.snapshot.lock() {
            Ok(guard) => guard.clone(),
            Err(p) => p.into_inner().clone(),
        }
    }

    /// True once the signal thread has its handlers installed and is waiting.
    pub fn ready(&self) -> bool {
        self.ready.load(Ordering::Relaxed)
    }

    #[must_use]
    pub fn signals_seen(&self) -> u64 {
        self.signals_seen.load(Ordering::Relaxed)
    }

    fn dump_prefix(seq: u64) -> String {
        format!("multitop-diag-{}-{seq:04}", std::process::id())
    }

    /// The tier that must beat a wedged loop: counters and phase first, then
    /// whatever snapshot exists, never touching a lock the loop could hold.
    pub fn write_signal_tier(&self, seq: u64, sig: &'static str) -> Option<PathBuf> {
        let snap = self.latest_snapshot();
        let body = self.render(sig, seq, snap.as_ref());
        let path = self
            .out_dir
            .join(format!("{}-{sig}-signal.txt", Self::dump_prefix(seq)));
        write_file(&path, &body).then_some(path)
    }

    /// The tier only the loop can produce, written by the loop itself on the
    /// poll after a request.
    pub fn write_state_tier(
        &self,
        seq: u64,
        sig: &'static str,
        snap: &Snapshot,
    ) -> Option<PathBuf> {
        let body = self.render(sig, seq, Some(snap));
        let path = self
            .out_dir
            .join(format!("{}-{sig}-state.txt", Self::dump_prefix(seq)));
        write_file(&path, &body).then_some(path)
    }

    /// Header + counters, shared by both tiers.
    fn render(&self, sig: &'static str, seq: u64, snap: Option<&Snapshot>) -> String {
        use std::fmt::Write as _;
        let uptime = self.start.elapsed().as_secs();
        let mut out = String::new();
        let _ = write!(
            out,
            "multitop diagnostic dump\n\
             version: {} | pid: {} | uptime: {uptime}s | trigger: {sig} | seq: {seq}\n\
             phase: {} | polls: {} | keys: {} | msgs applied: {} | drained: {}\n\
             signals seen: {} | handler ready: {}\n",
            env!("CARGO_PKG_VERSION"),
            std::process::id(),
            Phase::from_code(self.phase.load(Ordering::Relaxed)).name(),
            self.iter.load(Ordering::Relaxed),
            self.keys.load(Ordering::Relaxed),
            self.applied.load(Ordering::Relaxed),
            self.drained.load(Ordering::Relaxed),
            self.signals_seen(),
            self.ready()
        );
        match snap {
            Some(s) => {
                let _ = writeln!(out, "snapshot: captured");
                let _ = out.write_str(&render_snapshot(s));
            }
            None => {
                let _ = writeln!(out, "snapshot: none yet");
            }
        }
        out
    }
}

/// Install the diagnostic thread for this process. At most one instance ever
/// reacts, however often the loop is entered (an `event_loop` run in tests
/// counts as an entry).
pub fn install(diag: &Arc<Diag>) {
    static STARTED: Once = Once::new();
    STARTED.call_once(|| {
        let inner = Arc::clone(diag);
        let spawned = std::thread::Builder::new()
            .name("multitop-diag".into())
            .spawn(move || signal_thread(&inner));
        if let Err(e) = spawned {
            report(&format!("diag: could not start the diagnostic thread: {e}"));
        }
    });
}

fn signal_thread(diag: &Diag) {
    use signal_hook::consts::{SIGUSR1, SIGUSR2};
    let mut sigs = match signal_hook::iterator::Signals::new([SIGUSR1, SIGUSR2]) {
        Ok(s) => s,
        Err(e) => {
            report(&format!(
                "diag: could not subscribe to SIGUSR1/SIGUSR2: {e}"
            ));
            return;
        }
    };
    diag.ready.store(true, Ordering::Relaxed);
    for sig in sigs.forever() {
        let name = Diag::sig_name(sig);
        diag.note_signal(sig);
        let seq = diag.requested_seq.load(Ordering::Relaxed);
        match diag.write_signal_tier(seq, name) {
            Some(path) => report(&format!("diag: {name}: wrote {}", path.display())),
            None => report(&format!(
                "diag: {name}: dump requested but could not be written"
            )),
        }
    }
}

/// Say something about a dump, but never onto a terminal we are drawing on.
///
/// **Every** site with something to say about a dump goes through here; there
/// are five, and this was an `eprintln!` at each. stderr is the terminal the TUI
/// holds in raw mode inside the alternate screen, so each signal scribbled a
/// wrapped path across the operator's frame -- a tool for reading a display that
/// has stopped making sense, making it stop making sense.
///
/// The dump is a file; the file is the output. Redirected somewhere that is not
/// a terminal there is no frame to damage, and the line is worth having.
pub fn report(message: &str) {
    use std::io::IsTerminal as _;
    if std::io::stderr().is_terminal() {
        return;
    }
    eprintln!("{message}");
}

#[cfg(unix)]
fn write_file(path: &Path, body: &str) -> bool {
    use std::io::Write as _;
    use std::os::unix::fs::PermissionsExt;
    let mut f = match std::fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(path)
    {
        Ok(f) => f,
        Err(e) => {
            report(&format!("diag: cannot open {}: {e}", path.display()));
            return false;
        }
    };
    if let Err(e) = f.write_all(body.as_bytes()).and_then(|()| f.sync_all()) {
        report(&format!("diag: cannot write {}: {e}", path.display()));
        return false;
    }
    if let Err(e) = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)) {
        report(&format!("diag: cannot chmod {}: {e}", path.display()));
    }
    true
}

#[cfg(not(unix))]
fn write_file(_path: &Path, _body: &str) -> bool {
    false
}

/// The durable, redaction-safe picture of an app.
///
/// Build from the pieces the dump may legally know; `App` and `Panel` fields
/// that hold credentials never appear here.
#[must_use]
pub fn snapshot_app(app: &App, tasks: &Tasks) -> Snapshot {
    Snapshot {
        mode: format!("{:?}", app.mode),
        filter: app.filter_query.clone(),
        selected: app.selected_panel,
        panels: app
            .panels
            .iter()
            .map(|p| PanelDigest {
                host: p.server.host.clone(),
                mode: format!("{:?}", p.mode),
                state: format!("{:?}", p.upgrade_state),
                gen: p.gen,
                upgrade_gen: p.upgrade_gen,
                view_len: p.view.len(),
                ring_len: p.last_upgrade.len(),
                scroll: p.scroll_offset,
            })
            .collect(),
        in_flight: app.upgrades_in_flight(),
        quit: QuitFlags {
            armed: app.quit_armed,
            requested: app.should_quit,
        },
        vault_unlocked: app.vault_unlocked().is_some(),
        last_update: app.last_update,
        upgrade_started_at: app.upgrade_started_at,
        tasks: tasks.diag_liveness(),
        active_confirm: app.active_confirm().map(|c| format!("{c:?}")),
    }
}

fn render_snapshot(snapshot: &Snapshot) -> String {
    use std::fmt::Write as _;
    let mut out = String::new();
    let _ = writeln!(
        out,
        "mode: {} | filter: {:?} | selected: {} | in_flight: {} | quit_armed: {} | should_quit: {} | vault_unlocked: {} | active_confirm: {:?}",
        snapshot.mode,
        snapshot.filter,
        snapshot.selected,
        snapshot.in_flight,
        snapshot.quit.armed,
        snapshot.quit.requested,
        snapshot.vault_unlocked,
        snapshot.active_confirm
    );
    let _ = writeln!(
        out,
        "last_update: {} | upgrade_started_at: {}",
        fmt_opt(snapshot.last_update),
        fmt_opt(snapshot.upgrade_started_at)
    );
    let _ = writeln!(
        out,
        "tasks: monitors_alive {} | upgrades_alive {} | upgrades_done {}",
        snapshot.tasks.monitors_alive, snapshot.tasks.upgrades_alive, snapshot.tasks.upgrades_done
    );
    for (i, p) in snapshot.panels.iter().enumerate() {
        let _ = writeln!(
            out,
            "panel {i}: {} | {} | {} | gen {} (upgrade {}) | view {} | ring {} | scroll {}",
            p.host, p.mode, p.state, p.gen, p.upgrade_gen, p.view_len, p.ring_len, p.scroll
        );
    }
    out
}

fn fmt_opt(t: Option<u64>) -> String {
    t.map_or_else(|| "none".to_string(), |v| v.to_string())
}

#[cfg(test)]
#[path = "diag_tests.rs"]
mod tests;
