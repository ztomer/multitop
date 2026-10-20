//! What the Ops view draws: the machine's answers, in the shape it draws
//! them.
//!
//! Every type here is a fact the SERVER owns. The view reads a tool's
//! answer and draws it; it never re-derives one (DECISIONS #13): the pool's
//! state is `zfs.health`'s, the tunnel's verdict is `net.egress`'s, and the
//! machine's filesystems are `disk.usage`'s. What this module adds is the
//! SHAPE the answer had - the rows grouped, the sizes in one unit - not a
//! fact of its own.
//!
//! The rows that come from an INSTANCE carry the instance's name, because a
//! machine may run several roots (`.90` runs the storage root and the solver
//! root) and a row that does not say which root it came from cannot be
//! attributed to either.

/// The state a pool or a vdev is in when nothing is wrong with it.
pub const ONLINE: &str = "ONLINE";

/// A gibibyte: the unit the estate's card counts disk in.
const GIB: f64 = 1_073_741_824.0;
/// The gibibytes in a terabyte: the point where the card counts in
/// terabytes.
const GIB_PER_TIB: f64 = 1024.0;
/// The size under which a memory amount keeps its decimal: "of 2 GB" would
/// round a 1.9 GB machine up.
const SMALL_MEMORY_GB: f64 = 10.0;
/// A load average is published as a float and kept in thousandths.
const LOAD_SCALE: f64 = 1000.0;

/// One health check's last recorded verdict.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Check {
    pub name: String,
    pub ok: bool,
    pub rc: i64,
    pub ts: i64,
}

/// One cron job, as `cron.status` publishes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Job {
    pub label: String,
    /// The instance that schedules the job, when the server says: two roots
    /// on one machine may schedule the SAME label, and a job that does not
    /// say which root owns it cannot be told apart from its namesake.
    pub instance: Option<String>,
    pub last_start: Option<i64>,
    pub last_rc: Option<i64>,
    pub running_since: Option<i64>,
    pub alert_since: Option<i64>,
}

impl Job {
    /// Its last run failed, or it is alerting.
    #[must_use]
    pub fn in_trouble(&self) -> bool {
        self.last_rc.is_some_and(|rc| rc != 0) || self.alert_since.is_some()
    }
}

/// One container, as `docker.containers` publishes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Container {
    pub name: String,
    pub state: String,
    pub status: String,
    pub health: String,
    /// The instance the container belongs to, when the server says. The
    /// server resolves it: the compose project label is a Docker word, and
    /// the estate spells its stacks three ways.
    pub instance: Option<String>,
    /// The compose project the container runs under. Kept because it is the
    /// server's own marker and the fallback for a server that does not
    /// resolve the instance.
    pub compose_project: Option<String>,
}

impl Container {
    /// Unhealthy, or not running when it should be. A container outside any
    /// compose project that exited 0 is a finished one-shot (a `docker run`
    /// that did its job), not a stopped service - the same rule as the
    /// routines follower's.
    #[must_use]
    pub fn in_trouble(&self) -> bool {
        if self.health == "unhealthy" {
            return true;
        }
        if self.state == "running" {
            return false;
        }
        !(self.compose_project.is_none() && self.status.starts_with("Exited (0)"))
    }

    /// The stack the container belongs to: the server's `instance`, else the
    /// compose project label (an older server's only marker).
    #[must_use]
    pub fn stack(&self) -> &str {
        self.instance
            .as_deref()
            .or(self.compose_project.as_deref())
            .unwrap_or_default()
    }
}

/// One alert event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Alert {
    pub ts: i64,
    pub subject: String,
}

/// One member of a pool's vdev tree, as `zfs.health` publishes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Member {
    /// The vdev's name as the pool knows it (`sdb`, `mirror-0`, ...).
    pub name: String,
    /// `ONLINE`, `DEGRADED`, `REMOVED`, `FAULTED`, `UNAVAIL`, ...
    pub state: String,
}

impl Member {
    /// A member that is not ONLINE.
    #[must_use]
    pub fn is_faulted(&self) -> bool {
        self.state != ONLINE
    }
}

/// One ZFS pool, as `zfs.health` publishes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pool {
    /// The pool's name.
    pub name: String,
    /// `ONLINE`, `DEGRADED`, `SUSPENDED`, `UNAVAIL`, ...
    pub state: String,
    /// What ZFS says to do about it, when it says anything.
    pub action: String,
    /// ZFS's explanation, when it has one.
    pub msg: String,
    /// The vdev tree, the pool's children first.
    pub members: Vec<Member>,
}

impl Pool {
    /// The members that are not ONLINE.
    ///
    /// The pool's OWN row is not in the member list (it is the pool, not a
    /// vdev of it), so a DEGRADED `mirror-0` is a faulted member and is
    /// named - it is the level that says the pool lost redundancy.
    #[must_use]
    pub fn faulted_members(&self) -> Vec<&Member> {
        self.members.iter().filter(|m| m.is_faulted()).collect()
    }

    /// Whether the pool is whole: ONLINE itself, and every member ONLINE.
    ///
    /// The estate's rule, the one `check_zfs_pool` fails on and the tool's
    /// own summary uses: a pool that is not whole is invisible to every
    /// other view of the host (a SUSPENDED pool's I/O does not happen, so
    /// writes BLOCK rather than fail, and `disk.usage` still reports a
    /// plausible size for it).
    #[must_use]
    pub fn is_whole(&self) -> bool {
        self.state == ONLINE && self.faulted_members().is_empty()
    }
}

/// The verdict states `net.egress` publishes. They are the server's
/// vocabulary, and the view branches on them without inventing one.
pub const LEAK_CONFIRMED: &str = "confirmed";
pub const LEAK_LEAKED: &str = "leaked";
pub const LEAK_DISAGREEMENT: &str = "disagreement";
pub const LEAK_UNVERIFIED: &str = "unverified";
pub const LEAK_UNKNOWN: &str = "unknown";

/// How bad a verdict is, as the view's tone bands.
pub const VERDICT_OK: u8 = 0;
pub const VERDICT_UNSETTLED: u8 = 1;
pub const VERDICT_BAD: u8 = 2;

/// One tunnel a machine's instance declares, as `net.egress` publishes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tunnel {
    /// The stack that declares the tunnel, when the server says. A server
    /// that does not resolve the instance leaves it out, and the view says
    /// so rather than guessing which root owns the tunnel.
    pub instance: Option<String>,
    /// The tunnel's CONTAINER (`solver-vpn`).
    pub container: String,
    /// The exit country, when the verdict confirms one. A leak, a
    /// disagreement and an unverifiable answer are all "not a country", and
    /// `leak` is what tells them apart.
    pub country: Option<String>,
    /// The verdict: `confirmed`, `leaked`, `disagreement`, `unverified` or
    /// `unknown`.
    pub leak: String,
}

impl Tunnel {
    /// Whether the exit is where it belongs.
    #[must_use]
    pub fn is_confirmed(&self) -> bool {
        self.leak == LEAK_CONFIRMED
    }

    /// How bad the verdict is: a leak and a tunnel that is not running are
    /// faults; a disagreement and an unverifiable answer are not settled
    /// either way.
    #[must_use]
    pub fn severity(&self) -> u8 {
        match self.leak.as_str() {
            LEAK_CONFIRMED => VERDICT_OK,
            LEAK_LEAKED | LEAK_UNKNOWN => VERDICT_BAD,
            _ => VERDICT_UNSETTLED,
        }
    }
}

/// What `net.egress` answered: the machine's tunnels, and the server's own
/// line for the machine that declares none.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Egress {
    /// One row per tunnel, in the server's order.
    pub tunnels: Vec<Tunnel>,
    /// The server's report line, read when the machine has no tunnel at all.
    pub line: String,
}

/// One filesystem, as `disk.usage` publishes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mount {
    /// The mountpoint.
    pub mount: String,
    /// The device, or the ZFS dataset (`rpool/ROOT/ubuntu_abc`).
    pub device: String,
    /// The filesystem's type (`zfs`, `ext4`, `vfat`, ...).
    pub fstype: String,
    /// The filesystem's size in bytes.
    pub size: u64,
    /// The space an unprivileged writer may use, in bytes.
    pub avail: u64,
    /// The share used, as `df` prints it.
    pub percent: u64,
}

impl Mount {
    /// The ZFS pool this mount belongs to: `None` for anything that is not
    /// ZFS.
    #[must_use]
    pub fn zfs_pool(&self) -> Option<&str> {
        (self.fstype == "zfs").then(|| self.device.split('/').next().unwrap_or(&self.device))
    }

    /// Whether the mount is a boot or firmware partition, which is not the
    /// operator's to manage.
    #[must_use]
    pub fn is_boot(&self) -> bool {
        self.mount == "/boot" || self.mount.starts_with("/boot/")
    }
}

/// The machine's filesystems, as `disk.usage` publishes them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Disk {
    /// Every real mount on the machine.
    pub mounts: Vec<Mount>,
}

impl Disk {
    /// The filesystems that count towards the machine's capacity: each ZFS
    /// pool ONCE (its datasets share one pool's space, so counting them
    /// again would count the same terabytes twice), and the boot
    /// partitions out.
    ///
    /// The estate's card counts the same way, so the two clients show the
    /// same machine.
    #[must_use]
    pub fn counted(&self) -> Vec<&Mount> {
        let mut out: Vec<&Mount> = Vec::new();
        let mut pools: Vec<&str> = Vec::new();
        for m in &self.mounts {
            if m.is_boot() {
                continue;
            }
            if let Some(pool) = m.zfs_pool() {
                if pools.contains(&pool) {
                    continue;
                }
                pools.push(pool);
            }
            out.push(m);
        }
        out
    }

    /// The machine's capacity: the free space across the filesystems that
    /// count, and the share of them that is used. `None` when nothing
    /// counts - a total over no filesystems would print as zero, which is a
    /// reading nobody took.
    #[must_use]
    pub fn total(&self) -> Option<Capacity> {
        let counted = self.counted();
        if counted.is_empty() {
            return None;
        }
        let size = counted.iter().fold(0_u64, |a, m| a.saturating_add(m.size));
        let free = counted.iter().fold(0_u64, |a, m| a.saturating_add(m.avail));
        let percent = if size == 0 {
            0
        } else {
            let used = size.saturating_sub(free);
            u64::try_from((u128::from(used) * 100) / u128::from(size)).unwrap_or(100)
        };
        Some(Capacity {
            size,
            free,
            percent,
        })
    }
}

/// The machine's capacity, in one line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Capacity {
    /// The total size of the filesystems that count, in bytes.
    pub size: u64,
    /// The free space across them, in bytes.
    pub free: u64,
    /// The share used, as a percentage.
    pub percent: u64,
}

/// The machine's memory, as `host.facts` publishes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Memory {
    pub total_kib: u64,
    pub avail_kib: u64,
}

impl Memory {
    /// Whether the machine reported a total at all.
    #[must_use]
    pub const fn is_reported(&self) -> bool {
        self.total_kib > 0
    }
}

/// One row of the machine's instance declaration, as `host.facts` publishes
/// it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstanceRow {
    /// The instance's name (the root's own name).
    pub instance: String,
    /// The root, as declared.
    pub root: String,
    /// Whether the root is there on this machine.
    pub present: bool,
    /// Why this row is not an instance, or why its root is not there. The
    /// server's own words.
    pub problem: Option<String>,
}

/// The machine's instances: the declaration it published.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Instances {
    /// How many instances the declaration declares.
    pub declared: usize,
    /// Whether the declaration was read and understood. False means the
    /// machine's instances are UNKNOWN, which is not the same as "there is
    /// one".
    pub read: bool,
    /// Every row the machine knows about.
    pub rows: Vec<InstanceRow>,
}

impl Instances {
    /// The instances that are there.
    #[must_use]
    pub fn present(&self) -> Vec<&InstanceRow> {
        self.rows.iter().filter(|r| r.present).collect()
    }

    /// The rows that are findings: a rejected row, or a root that is not
    /// there.
    #[must_use]
    pub fn problems(&self) -> Vec<&InstanceRow> {
        self.rows.iter().filter(|r| r.problem.is_some()).collect()
    }
}

/// The machine's identity and load, as `host.facts` publishes them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Facts {
    /// The logical CPUs the machine runs, so a load average can be read as a
    /// share of it.
    pub ncpu: i64,
    /// The 1, 5 and 15 minute load averages, in thousandths of a load (70 is
    /// 0.07).
    ///
    /// Whole numbers: the view's own types are comparable, and a float is
    /// not. A load is a reading, not an identity.
    pub load: Vec<i64>,
    /// The machine's memory.
    pub mem: Memory,
    /// The machine's instances, when the server publishes the declaration.
    pub instances: Option<Instances>,
}

/// A disk amount in the words the estate's card counts in: gigabytes, and
/// terabytes from a thousand of them.
///
/// The card's own units, so the two clients say the same number about the
/// same machine.
#[must_use]
pub fn amount(bytes: u64) -> String {
    let gb = gib(bytes);
    if gb >= GIB_PER_TIB {
        format!("{:.1} TB", gb / GIB_PER_TIB)
    } else {
        format!("{gb:.0} GB")
    }
}

/// A memory amount, in the card's words: one decimal under ten gigabytes,
/// whole gigabytes above.
#[must_use]
pub fn memory_amount(kib: u64) -> String {
    let gb = gib(kib.saturating_mul(1024));
    if gb < SMALL_MEMORY_GB {
        format!("{gb:.1} GB")
    } else {
        format!("{gb:.0} GB")
    }
}

/// A load average in thousandths of a load.
///
/// The float is written out and read back rather than cast: the house does
/// not silence the cast lints, and a load that is not a number is no load.
#[must_use]
pub fn load_milli(v: f64) -> i64 {
    let scaled = (v * LOAD_SCALE).round();
    if !scaled.is_finite() || scaled <= 0.0 {
        return 0;
    }
    format!("{scaled:.0}").parse().unwrap_or(i64::MAX)
}

/// A load average in thousandths, as the view prints it.
#[must_use]
pub fn load_text(milli: i64) -> String {
    format!("{:.2}", f64_of_i64(milli) / LOAD_SCALE)
}

/// An integer as f64: exact below 2^53, rounded above it.
fn f64_of_i64(n: i64) -> f64 {
    let hi = u32::try_from(n >> 32).unwrap_or(u32::MAX);
    let lo = u32::try_from(n & 0xFFFF_FFFF).unwrap_or(u32::MAX);
    f64::mul_add(f64::from(hi), 4_294_967_296.0, f64::from(lo))
}

/// A byte count as gibibytes, without the lossy cast the lints refuse.
fn gib(bytes: u64) -> f64 {
    let hi = u32::try_from(bytes >> 32).unwrap_or(u32::MAX);
    let lo = u32::try_from(bytes & 0xFFFF_FFFF).unwrap_or(u32::MAX);
    f64::mul_add(f64::from(hi), 4_294_967_296.0, f64::from(lo)) / GIB
}
