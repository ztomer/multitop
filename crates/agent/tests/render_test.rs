use multitop_agent::color::{strip_ansi, ANSI};
use multitop_agent::proc::{Proc, Usage};
use multitop_agent::render::*;

fn usage(total: u64, used: u64, pct: f64) -> Usage {
    Usage { total, used, pct }
}

fn proc(pid: u32, name: &str, cpu: f64, mem: u64) -> Proc {
    Proc {
        pid,
        name: name.to_string(),
        cpu,
        mem,
    }
}

fn snap() -> Snapshot {
    Snapshot {
        host: "h".into(),
        ..Default::default()
    }
}

fn find(out: &[String], needle: &str) -> Vec<String> {
    out.iter().filter(|l| l.contains(needle)).cloned().collect()
}

fn labeled(out: &[String], label: &str) -> Vec<String> {
    let tag = format!("{}{}{}", ANSI.bold, label, ANSI.reset);
    out.iter().filter(|l| l.contains(&tag)).cloned().collect()
}

#[test]
fn host_line_is_fullwidth() {
    assert!(render(&snap(), 80, 0, 50, &ANSI)[0].contains('\u{ff48}'));
}

#[test]
fn host_line_never_overflows_narrow_panel() {
    let s = Snapshot {
        host: "a-very-long-hostname (10.0.0.1)".into(),
        ..snap()
    };
    assert!(strip_ansi(&render(&s, 20, 0, 4, &ANSI)[0]).contains('\u{ff41}'));
}

#[test]
fn single_core_uses_aggregate_bar() {
    let s = Snapshot {
        cpu_pct: 42.0,
        cpu_mhz: Some(3600.0),
        proc_names: Vec::new(),
        ..snap()
    };
    let out = render(&s, 80, 0, 50, &ANSI);
    assert!(out[1].contains("CPU") && out[1].contains("42%") && out[1].contains('['));
}

#[test]
fn dual_core_shows_per_core_cells() {
    let s = Snapshot {
        cores: vec![(0, 75.0, None), (1, 25.0, None)],
        ..snap()
    };
    let out = render(&s, 80, 0, 50, &ANSI);
    for want in ["CPU", "0:", "1:", "75%", "25%"] {
        assert!(out[1].contains(want), "missing {want} in {:?}", out[1]);
    }
}

#[test]
fn many_cores_wrap_to_multiple_rows() {
    let s = Snapshot {
        cores: (0..8).map(|i| (i, i as f64 * 10.0, None)).collect(),
        ..snap()
    };
    let out = render(&s, 40, 0, 20, &ANSI);
    assert!(
        out.iter()
            .filter(|l| l.contains(':') && l.contains('%'))
            .count()
            >= 2
    );
}

#[test]
fn mem_shown_when_total_present() {
    let s = Snapshot {
        mem: usage(1 << 31, 1 << 30, 50.0),
        ..snap()
    };
    let rows = labeled(&render(&s, 80, 0, 50, &ANSI), "MEM");
    assert_eq!(rows.len(), 1);
    assert!(rows[0].contains("50%") && rows[0].contains("GiB"));
}

#[test]
fn mem_omitted_when_zero() {
    let s = Snapshot {
        disk: usage(1 << 40, 1 << 38, 80.0),
        ..snap()
    };
    assert!(labeled(&render(&s, 80, 0, 50, &ANSI), "MEM").is_empty());
}

#[test]
fn disk_shown_when_total_present() {
    let s = Snapshot {
        disk: usage(1 << 40, 1 << 38, 80.0),
        ..snap()
    };
    let rows = labeled(&render(&s, 80, 0, 50, &ANSI), "DSK");
    assert_eq!(rows.len(), 1);
    assert!(rows[0].contains("80%") && rows[0].contains("TiB"));
}

#[test]
fn disk_omitted_when_zero() {
    let s = Snapshot {
        mem: usage(1 << 31, 1 << 30, 50.0),
        ..snap()
    };
    assert!(labeled(&render(&s, 80, 0, 50, &ANSI), "DSK").is_empty());
}

#[test]
fn net_shown_when_traffic() {
    let s = Snapshot {
        rx_rate: 2e6,
        tx_rate: 3e6,
        ..snap()
    };
    let rows = find(&render(&s, 80, 0, 50, &ANSI), "NET");
    assert_eq!(rows.len(), 1);
    assert!(rows[0].contains('\u{2191}') && rows[0].contains('\u{2193}'));
}

#[test]
fn net_omitted_when_idle() {
    let s = Snapshot {
        rx_rate: 500.0,
        tx_rate: 500.0,
        ..snap()
    };
    assert!(find(&render(&s, 80, 0, 50, &ANSI), "NET").is_empty());
}

#[test]
fn net_threshold_is_strictly_above_1k() {
    assert!(!shows_net(1024.0, 1024.0));
    assert!(shows_net(1025.0, 0.0));
    assert!(shows_net(0.0, 1025.0));
}

#[test]
fn procs_are_listed_with_header() {
    let s = Snapshot {
        procs: vec![
            proc(100, "python3", 2.5, 45_000),
            proc(200, "bash", 0.5, 12_000),
        ],
        ..snap()
    };
    let all = render(&s, 80, 0, 50, &ANSI).join("\n");
    assert!(all.contains("python3") && all.contains("bash") && all.contains("PID"));
}

#[test]
fn empty_proc_list_draws_no_table() {
    let out = render(&snap(), 80, 0, 50, &ANSI);
    assert!(!out.iter().any(|l| l.contains("PID")));
}

#[test]
fn long_proc_name_is_truncated() {
    let s = Snapshot {
        procs: vec![proc(1, "verylongprocessnameishere", 1.0, 1000)],
        ..snap()
    };
    assert!(render(&s, 80, 0, 50, &ANSI).join("\n").contains("..."));
}

#[test]
fn truncated_name_fits_exactly() {
    assert_eq!(truncate_name("abcdefghij", 6), "abc...");
    assert_eq!(truncate_name("abc", 6), "abc");
    assert_eq!(truncate_name("abcdef", 6), "abc...");
    assert_eq!(truncate_name("abcde", 6), "abcde");
}

#[test]
fn truncate_name_is_char_safe() {
    assert_eq!(
        truncate_name("\u{4f60}\u{597d}\u{4e16}\u{754c}\u{ff01}", 4),
        "\u{4f60}..."
    );
}

#[test]
fn hot_proc_is_highlighted() {
    let s = Snapshot {
        procs: vec![proc(1, "hungry", 95.0, 1000)],
        ..snap()
    };
    let out = render(&s, 80, 0, 50, &ANSI);
    assert!(out
        .iter()
        .find(|l| l.contains("hungry"))
        .unwrap()
        .contains(ANSI.yellow));
}

#[test]
fn highlight_threshold_is_ten_percent() {
    for (cpu, hot) in [(9.9, false), (10.0, true)] {
        let s = Snapshot {
            procs: vec![proc(1, "solo", cpu, 0)],
            ..snap()
        };
        let out = render(&s, 80, 0, 50, &ANSI);
        let line = out.iter().find(|l| l.contains("solo")).unwrap();
        assert_eq!(line.contains(ANSI.yellow), hot, "cpu={cpu}");
    }
}

#[test]
fn mem_and_dsk_rows_have_constant_width() {
    let a = Snapshot {
        mem: usage(1 << 31, 1 << 30, 50.0),
        ..snap()
    };
    let b = Snapshot {
        mem: usage(1 << 30, 1 << 20, 12.5),
        ..snap()
    };
    let wa = strip_ansi(&labeled(&render(&a, 80, 0, 48, &ANSI), "MEM")[0])
        .chars()
        .count();
    let wb = strip_ansi(&labeled(&render(&b, 80, 0, 48, &ANSI), "MEM")[0])
        .chars()
        .count();
    assert_eq!(wa, wb);
}

#[test]
fn mem_and_dsk_rows_match_each_other() {
    let s = Snapshot {
        mem: usage(1 << 31, 1 << 30, 50.0),
        disk: usage(1 << 40, 1 << 38, 80.0),
        ..snap()
    };
    let out = render(&s, 80, 0, 48, &ANSI);
    assert_eq!(
        strip_ansi(&labeled(&out, "MEM")[0]).chars().count(),
        strip_ansi(&labeled(&out, "DSK")[0]).chars().count()
    );
}

#[test]
fn percentage_is_right_aligned() {
    let s = Snapshot {
        mem: usage(1 << 31, 1 << 30, 50.0),
        ..snap()
    };
    assert!(strip_ansi(&labeled(&render(&s, 80, 0, 48, &ANSI), "MEM")[0]).contains("  50%"));
}

#[test]
fn proc_rows_stay_aligned_across_all_size_magnitudes() {
    let sizes = [
        0u64,
        999,
        1024,
        1000 * 1024,
        1023 * 1024 + 1023,
        1024 * 1024,
        1005 * 1024 * 1024,
        1 << 30,
        1023 * (1 << 30),
        1 << 40,
    ];
    let procs: Vec<Proc> = sizes
        .iter()
        .enumerate()
        .map(|(i, &m)| proc(i as u32 + 1, "proc", i as f64, m))
        .collect();
    let s = Snapshot { procs, ..snap() };

    for cols in [60usize, 80, 120] {
        let out = render(&s, cols, 0, bar_len_for(cols), &ANSI);
        let widths: Vec<usize> = out
            .iter()
            .skip_while(|l| !l.contains("PID"))
            .skip(1)
            .map(|l| strip_ansi(l).chars().count())
            .collect();
        assert!(widths.len() >= 5);
        assert!(
            widths.windows(2).all(|w| w[0] == w[1]),
            "cols={cols} widths={widths:?}"
        );
    }
}
