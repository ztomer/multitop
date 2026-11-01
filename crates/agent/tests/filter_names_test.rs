//! The process-name list the filter searches: what it covers, its order and
//! bound, that it is rebuilt rather than accumulated, and that a busy host's
//! every distinct name reaches it and the wire.
//!
//! Split from `sampling_test` for length; the subject is one list.

// A test crate, said where clippy reads it: the restriction lints
// (`unwrap_used`, `expect_used`, `panic`) are policy for production code and
// exempt for test code (clippy.toml), and an integration test is test code
// through and through -- helpers included.
#![cfg(test)]

use multitop_agent::proc::{self, ProcSampler};
use multitop_agent::SortBy;

/// The table `top_into` fills, as a fresh list for a test to hold.
fn top_of(s: &mut ProcSampler, elapsed: f64, n: usize, sort: SortBy) -> Vec<proc::Proc> {
    let mut rows = Vec::new();
    s.top_into(elapsed, n, sort, &mut rows);
    rows
}

#[test]
fn the_name_list_covers_processes_the_table_had_no_room_for() {
    // The point of the list: the table is capped to what fits the pane, and the
    // filter must not be capped with it. This host is running many more than
    // three processes, so a three-row table proves the two lists differ.
    let mut sampler = ProcSampler::new();
    sampler.prime();
    let table = top_of(&mut sampler, 1.0, 1, SortBy::Cpu);
    let mut names = Vec::new();
    sampler.fill_names(&mut names);

    assert!(table.len() <= 1, "the table ignored its budget");
    // Stated as a precondition rather than assumed: a host running one process
    // has nothing for the list to look past, and this would be vacuous there
    // rather than wrong.
    if sampler.scanned.len() > 1 {
        assert!(
            names.len() > table.len(),
            "the name list is no bigger than the table it exists to look past: \
             {} names, {} rows, {} processes scanned",
            names.len(),
            table.len(),
            sampler.scanned.len()
        );
    }
    for p in &table {
        assert!(
            names.iter().any(|n| *n == p.name.as_str()),
            "a process in the table is missing from the list: {}",
            p.name
        );
    }
}

#[test]
fn the_name_list_is_deduplicated_sorted_and_bounded() {
    // Deduplicated because a host with forty `nginx` workers has one name worth
    // sending; sorted so an unchanged host produces the same bytes each tick
    // rather than a set's arbitrary order.
    let mut sampler = ProcSampler::new();
    sampler.prime();
    let mut names = Vec::new();
    sampler.fill_names(&mut names);

    let mut unique = names.clone();
    unique.dedup();
    assert_eq!(unique, names, "the list repeats a name");

    let mut sorted = names.clone();
    sorted.sort_unstable();
    assert_eq!(sorted, names, "the list is not in a stable order");

    multitop_testassert::assert_not_empty!(names, "this process is running, so something scanned");
    assert!(
        names.len() <= multitop_agent::consts::MAX_FILTER_NAMES,
        "the list is unbounded: {} names",
        names.len()
    );
    assert!(
        names.iter().all(|n| !n.is_empty()),
        "an empty name reached the list"
    );
}

#[test]
fn the_name_list_is_rebuilt_each_scan_rather_than_accumulating() {
    // It is a buffer reused between ticks. If it were appended to, a process
    // that exited would stay findable forever.
    let mut sampler = ProcSampler::new();
    sampler.prime();
    let mut names = Vec::new();
    let _ = top_of(&mut sampler, 1.0, 3, SortBy::Cpu);
    sampler.fill_names(&mut names);
    let first = names.len();
    let _ = top_of(&mut sampler, 1.0, 3, SortBy::Cpu);
    sampler.fill_names(&mut names);
    let second = names.len();
    // Processes come and go on a real host, so this is a band rather than an
    // equality -- but an accumulating list would roughly double.
    assert!(
        second < first * 2,
        "the list grew from {first} to {second}; it is accumulating, not rebuilding"
    );
}

#[test]
fn every_distinct_name_on_a_busy_desktop_reaches_the_filter_and_the_wire() {
    // The filter matches a panel by this list (`multitop::filter`), so a name
    // left off it is a process `/` cannot find. The cap was 512, and the Mac
    // this is developed on runs 778 distinct names (2026-11-01, roadmap M2):
    // everything past the 512th in sort order was unfindable. 2,000 here --
    // past the desktop by half again -- at the kernel's 15-byte name length,
    // the longest a Linux host sends.
    let count = 2_000;
    let mut sampler = ProcSampler::new();
    sampler.scanned = (0..count)
        .map(|i| proc::RawProcStat {
            pid: u32::try_from(i + 1).expect("small pid"),
            comm: multitop_agent::proc_name::ProcName::default(),
            stat_comm: format!("worker-name{i:04}").as_str().into(),
            ticks: 0,
            starttime: 0,
            rss_pages: 0,
        })
        .collect();
    let mut names = Vec::new();
    sampler.fill_names(&mut names);
    assert_eq!(names.len(), count, "the list dropped names");
    assert_eq!(
        names
            .last()
            .map(multitop_agent::proc_name::ProcName::as_str),
        Some("worker-name1999")
    );

    let snap = multitop_agent::render::Snapshot {
        proc_names: names.clone(),
        ..multitop_agent::render::Snapshot::default()
    };
    let packet =
        multitop_agent::proto::encode_packet(&multitop_agent::proto::Payload::Monitor(snap));
    let Some(multitop_agent::proto::Payload::Monitor(got)) =
        multitop_agent::proto::decode_packet(&packet)
    else {
        panic!("the monitor packet did not decode");
    };
    assert_eq!(got.proc_names, names, "names were lost on the wire");
}
