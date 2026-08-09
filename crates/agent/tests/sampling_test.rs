//! The sampling layer: `/proc` parsers on their fixture text, and the live
//! samplers run against whatever host the suite is on.
//!
//! The parsers are the part that has to be right on a machine the developer
//! is not sitting at, so they are driven from fixtures rather than from this
//! host's real `/proc`. The samplers are then called for real, which is what
//! proves the fallback chain (read `/proc`, else ask the platform) terminates
//! and produces a value on a host with no `/proc` at all.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use multitop_agent::fetch;
use multitop_agent::proc::{self, CpuTimes, ProcSampler, Usage};
use multitop_agent::SortBy;

// ------------------------------------------------------------------ uptime

#[test]
fn uptime_drops_the_leading_units_that_are_zero() {
    assert_eq!(fetch::format_uptime(0), "0m");
    assert_eq!(fetch::format_uptime(59), "0m");
    assert_eq!(fetch::format_uptime(60), "1m");
    assert_eq!(fetch::format_uptime(3600), "1h 0m");
    assert_eq!(fetch::format_uptime(3600 + 120), "1h 2m");
    assert_eq!(fetch::format_uptime(86400), "1d 0h 0m");
    assert_eq!(
        fetch::format_uptime(86400 * 3 + 3600 * 4 + 60 * 5),
        "3d 4h 5m"
    );
}

#[test]
fn proc_uptime_takes_the_first_field_only() {
    // `/proc/uptime` is "<up> <idle>"; the idle column must not be read.
    assert_eq!(
        fetch::parse_proc_uptime("350735.47 234388.90\n"),
        Some(350_735)
    );
    assert_eq!(fetch::parse_proc_uptime("0.00 0.00"), Some(0));
}

#[test]
fn an_absent_or_malformed_proc_uptime_yields_nothing() {
    assert_eq!(fetch::parse_proc_uptime(""), None);
    assert_eq!(fetch::parse_proc_uptime("   \n"), None);
    assert_eq!(fetch::parse_proc_uptime("not-a-number 1.0"), None);
}

// -------------------------------------------------------------- os-release

#[test]
fn os_release_prefers_pretty_name() {
    let s =
        "NAME=\"Debian GNU/Linux\"\nPRETTY_NAME=\"Debian GNU/Linux 12 (bookworm)\"\nID=debian\n";
    assert_eq!(
        fetch::parse_os_release(s).as_deref(),
        Some("Debian GNU/Linux 12 (bookworm)")
    );
}

#[test]
fn os_release_falls_back_to_name_when_pretty_name_is_absent() {
    assert_eq!(
        fetch::parse_os_release("ID=alpine\nNAME=\"Alpine Linux\"\n").as_deref(),
        Some("Alpine Linux")
    );
}

#[test]
fn an_os_release_with_neither_field_yields_nothing() {
    assert_eq!(fetch::parse_os_release("ID=weird\nVERSION_ID=1\n"), None);
    assert_eq!(fetch::parse_os_release(""), None);
    // Present but empty is the same as absent, not an empty distro name.
    assert_eq!(
        fetch::parse_os_release("PRETTY_NAME=\"\"\nNAME=\"\"\n"),
        None
    );
}

// ------------------------------------------------------------- host model

#[test]
fn a_vendor_that_repeats_itself_in_the_product_is_not_doubled() {
    assert_eq!(
        fetch::dmi_model("Dell Inc.", "Dell Inc. XPS 13").as_deref(),
        Some("Dell Inc. XPS 13")
    );
}

#[test]
fn vendor_and_product_are_joined_when_the_product_stands_alone() {
    assert_eq!(
        fetch::dmi_model("LENOVO\n", " 20XW private\n").as_deref(),
        Some("LENOVO 20XW private")
    );
}

#[test]
fn a_missing_product_name_yields_nothing_whatever_the_vendor_says() {
    assert_eq!(fetch::dmi_model("QEMU", ""), None);
    assert_eq!(fetch::dmi_model("", "  \n"), None);
    // Vendor unreadable but product present: the product alone is the answer.
    assert_eq!(
        fetch::dmi_model("", "MacBookPro18,3").as_deref(),
        Some("MacBookPro18,3")
    );
}

// --------------------------------------------------------------- cpu model

#[test]
fn cpuinfo_reports_the_model_and_the_core_count() {
    let s = "processor\t: 0\nmodel name\t: Intel(R) Core(TM) i7-8700\n\
             processor\t: 1\nmodel name\t: Intel(R) Core(TM) i7-8700\n";
    assert_eq!(
        fetch::parse_cpuinfo(s).as_deref(),
        Some("Intel(R) Core(TM) i7-8700 (2)")
    );
}

#[test]
fn arm_kernels_spell_the_model_differently_and_are_still_read() {
    assert_eq!(
        fetch::parse_cpuinfo("Hardware\t: BCM2835\nprocessor\t: 0\n").as_deref(),
        Some("BCM2835 (1)")
    );
    assert_eq!(
        fetch::parse_cpuinfo("Processor\t: ARMv7 rev 3\n").as_deref(),
        // No `processor` lines at all still has to report at least one core.
        Some("ARMv7 rev 3 (1)")
    );
}

#[test]
fn a_cpuinfo_without_a_model_yields_nothing() {
    assert_eq!(
        fetch::parse_cpuinfo("processor\t: 0\nbogomips\t: 48.00\n"),
        None
    );
    assert_eq!(fetch::parse_cpuinfo(""), None);
    // Lines with no colon are skipped rather than panicking.
    assert_eq!(fetch::parse_cpuinfo("garbage\n\n"), None);
}

// ---------------------------------------------------------- live samplers

#[test]
fn every_fetch_field_is_populated_on_this_host() {
    let snap = fetch::sample_fetch("testhost");
    assert!(snap.user_host.ends_with("@testhost"));
    assert!(!snap.agent_version.is_empty());
    // Each of these has a "give up" answer; none may come back blank, because
    // a blank field renders as a missing row rather than an honest "unknown".
    for (field, val) in [
        ("os", &snap.os),
        ("kernel", &snap.kernel),
        ("uptime", &snap.uptime),
        ("host_model", &snap.host_model),
        ("cpu_model", &snap.cpu_model),
        ("memory_str", &snap.memory_str),
        ("disk_str", &snap.disk_str),
    ] {
        assert!(!val.is_empty(), "{field} came back empty");
    }
}

#[test]
fn the_individual_samplers_each_answer_without_proc() {
    assert!(!fetch::sample_os().is_empty());
    assert!(!fetch::sample_kernel().is_empty());
    assert!(!fetch::sample_uptime().is_empty());
    assert!(!fetch::sample_host_model().is_empty());
    assert!(!fetch::sample_cpu_model().is_empty());
}

#[test]
fn reading_a_pseudofile_that_is_not_there_is_not_an_error() {
    assert_eq!(proc::read_proc("/proc/definitely/not/here"), "");
    let mut buf = [0u8; 64];
    assert_eq!(
        proc::read_proc_bytes("/proc/definitely/not/here", &mut buf),
        0
    );
    let mut s = String::new();
    assert!(!proc::read_proc_into("/proc/definitely/not/here", &mut s));
}

#[test]
fn a_real_file_is_read_whole_through_every_reader() {
    // Bigger than the 1 KiB chunk `read_proc_into` loops over, so the loop
    // runs more than once.
    let body = "x".repeat(4096);
    let path = std::env::temp_dir().join("multitop-agent-read-proc-test");
    std::fs::write(&path, &body).unwrap();

    assert_eq!(proc::read_proc(&path), body);

    let mut into = String::new();
    assert!(proc::read_proc_into(&path, &mut into));
    assert_eq!(into, body);

    let mut buf = [0u8; 4096];
    let n = proc::read_proc_bytes(&path, &mut buf);
    assert!(n > 0 && n <= 4096);

    std::fs::remove_file(&path).unwrap();
}

#[test]
fn an_empty_file_reads_as_empty_rather_than_as_a_failure() {
    let path = std::env::temp_dir().join("multitop-agent-read-proc-empty");
    std::fs::write(&path, "").unwrap();
    let mut into = String::new();
    // `read_proc_into` reports "got nothing", which is what makes the caller
    // fall through to the platform sampler.
    assert!(!proc::read_proc_into(&path, &mut into));
    assert!(into.is_empty());
    std::fs::remove_file(&path).unwrap();
}

#[test]
fn cpu_memory_disk_and_net_all_answer_on_this_host() {
    let cpu = proc::get_cpu_stat();
    // Either `/proc/stat` or the platform sampler answered; a total of zero
    // with no cores would mean neither did.
    assert!(cpu.aggregate.total > 0 || !cpu.cores.is_empty());

    let mem = proc::get_memory();
    assert!(mem.total > 0, "no memory total from either source");
    assert!(mem.pct >= 0.0 && mem.pct <= 100.0);

    let disk = proc::get_disk();
    assert!(disk.total > 0, "no disk total from statvfs");
    assert!(disk.used <= disk.total);

    // Net counters are monotonic totals; a host with no traffic is legal, so
    // this only asserts the call returns.
    let net = proc::get_net();
    let _ = net.rx.saturating_add(net.tx);

    let _ = proc::get_core_temps();
}

#[test]
fn statvfs_answers_for_root_and_declines_for_nonsense() {
    let (total, free) = proc::statvfs_bytes("/").expect("root filesystem must be stattable");
    assert!(total > 0);
    assert!(free <= total);
    assert_eq!(proc::statvfs_bytes("/no/such/path/at/all"), None);
    // An interior NUL cannot become a C string.
    assert_eq!(proc::statvfs_bytes("/tmp\0/x"), None);
}

#[test]
fn a_hostname_is_always_produced() {
    let h = proc::hostname();
    assert!(!h.is_empty());
    // With an explicit address the header carries it verbatim.
    assert_eq!(proc::host_info(Some("10.0.0.9")), format!("{h} (10.0.0.9)"));
    // An empty address is treated as absent, so the primary-IP lookup runs.
    let derived = proc::host_info(Some(""));
    assert!(derived.starts_with(&h));
    // And with no hint at all the same lookup decides whether to append.
    assert!(proc::host_info(None).starts_with(&h));
}

#[test]
fn the_primary_address_is_either_a_real_route_or_nothing() {
    // Never loopback or unspecified: those are what the caller wants
    // suppressed, so the header does not claim 127.0.0.1 is the host.
    if let Some(ip) = proc::primary_ip() {
        assert_ne!(ip, "127.0.0.1");
        assert_ne!(ip, "0.0.0.0");
    }
}

// ------------------------------------------------------------ proc sampler

#[test]
fn the_sampler_ranks_this_hosts_processes() {
    let mut sampler = ProcSampler::new();
    sampler.prime();
    let by_cpu = sampler.top(1.0, 5, SortBy::Cpu);
    assert!(by_cpu.len() <= 5);
    for w in by_cpu.windows(2) {
        assert!(w[0].cpu >= w[1].cpu, "cpu order broken: {w:?}");
    }

    let by_mem = sampler.top(1.0, 5, SortBy::Mem);
    assert!(by_mem.len() <= 5);
    for w in by_mem.windows(2) {
        assert!(w[0].mem >= w[1].mem, "mem order broken: {w:?}");
    }

    // A zero window cannot produce a rate, so every process reads as idle
    // rather than as a division by zero.
    for p in sampler.top(0.0, 5, SortBy::Cpu) {
        assert_eq!(p.cpu, 0.0);
    }

    // Asking for none is not an error, and asking for more than exist just
    // returns what exists.
    assert!(sampler.top(1.0, 0, SortBy::Cpu).is_empty());
    assert!(sampler.top(1.0, 100_000, SortBy::Mem).len() < 100_000);
}

#[test]
fn a_sampler_that_was_never_primed_still_reports() {
    // Without a baseline every rate is zero, which is the honest answer for a
    // first frame rather than a spike.
    let mut sampler = ProcSampler::new();
    for p in sampler.top(2.0, 3, SortBy::Cpu) {
        assert_eq!(p.cpu, 0.0);
    }
}

// -------------------------------------------------------------- pure maths

#[test]
fn busy_percentage_is_the_non_idle_share_of_the_window() {
    let prev = CpuTimes {
        total: 1000,
        idle: 800,
    };
    let curr = CpuTimes {
        total: 1100,
        idle: 850,
    };
    assert!((curr.pct_since(&prev) - 50.0).abs() < 1e-9);
}

#[test]
fn a_window_in_which_nothing_moved_is_zero_percent_busy() {
    let t = CpuTimes {
        total: 1000,
        idle: 800,
    };
    assert_eq!(t.pct_since(&t), 0.0);
    // Counters that went backwards (a reboot between samples) saturate to
    // zero rather than wrapping to a huge busy figure.
    let older = CpuTimes {
        total: 9999,
        idle: 9999,
    };
    assert_eq!(t.pct_since(&older), 0.0);
}

#[test]
fn usage_of_a_zero_sized_thing_is_zero_percent_not_a_nan() {
    let u = Usage::new(0, 0);
    assert_eq!(u.pct, 0.0);
    assert!(!u.pct.is_nan());
}
