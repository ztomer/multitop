use multitop_agent::color::{strip_ansi, ANSI};
use multitop_agent::proc::{Proc, Usage};
use multitop_agent::render::*;

/// Rows the rendered frame will occupy, composed from the two pieces the agent
/// itself uses to size a frame.
///
/// A helper here rather than in the library: nothing in the agent needs the sum
/// — `Chrome::proc_budget` is what production sizes with — so exporting it made
/// a public function whose only callers were these tests. What the tests are
/// for is that the sum agrees with what `render` emits.
fn frame_height(snap: &Snapshot, cols: usize, lines: usize) -> usize {
    let chrome = Chrome::of(snap, cols, lines);
    chrome.height() + chrome.table_height(snap.procs.len())
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

fn full(cores: usize, procs: usize, cols: usize) -> Snapshot {
    let _ = cols;
    Snapshot {
        host: "host (10.0.0.1)".into(),
        cores: (0..cores).map(|i| (i, 50.0, None)).collect(),
        mem: Usage {
            total: 1 << 31,
            used: 1 << 30,
            pct: 50.0,
        },
        disk: Usage {
            total: 1 << 40,
            used: 1 << 38,
            pct: 80.0,
        },
        rx_rate: 100_000.0,
        tx_rate: 100_000.0,
        procs: (0..procs as u32)
            .map(|i| proc(i, "proc", 1.0, 1000))
            .collect(),
        ..Default::default()
    }
}

#[test]
fn proc_rows_stay_aligned_across_pid_and_cpu_ranges() {
    let procs: Vec<Proc> = [(1u32, 0.0), (99, 9.9), (99999, 100.0), (4194304, 7.5)]
        .iter()
        .map(|&(pid, cpu)| proc(pid, "name", cpu, 1 << 20))
        .collect();
    let s = Snapshot { procs, ..snap() };
    let out = render(&s, 60, 0, bar_len_for(60), &ANSI);
    let widths: Vec<usize> = out
        .iter()
        .skip_while(|l| !l.contains("PID"))
        .map(|l| strip_ansi(l).chars().count())
        .collect();
    assert!(widths.windows(2).all(|w| w[0] == w[1]), "{widths:?}");
}

#[test]
fn two_column_rows_align() {
    let procs: Vec<Proc> = (1..=6)
        .map(|i| {
            proc(
                i * 1000,
                &format!("name{i}"),
                i as f64 * 3.0,
                i as u64 * 1_000_000,
            )
        })
        .collect();
    let s = Snapshot { procs, ..snap() };
    let out = render(&s, 100, 0, 68, &ANSI);
    let rows: Vec<usize> = out
        .iter()
        .filter(|l| l.contains('\u{2502}'))
        .map(|l| strip_ansi(l).chars().count())
        .collect();
    assert!(rows.len() >= 3);
    assert!(rows.windows(2).all(|w| w[0] == w[1]), "{rows:?}");
}

#[test]
fn core_grid_cells_are_fixed_width() {
    let s = Snapshot {
        cores: (0..16).map(|i| (i, (i * 7 % 100) as f64, None)).collect(),
        ..snap()
    };
    let out = render(&s, 100, 0, 48, &ANSI);
    let rows: Vec<String> = out
        .iter()
        .filter(|l| l.contains(':') && l.contains('%'))
        .map(|l| strip_ansi(l))
        .collect();
    assert!(rows.len() >= 2);
    let widths: Vec<usize> = rows[..rows.len() - 1]
        .iter()
        .map(|r| r.chars().count())
        .collect();
    assert!(widths.windows(2).all(|w| w[0] == w[1]), "{widths:?}");
}

#[test]
fn core_grid_indices_are_right_aligned() {
    let s = Snapshot {
        cores: (0..12).map(|i| (i, 5.0, None)).collect(),
        ..snap()
    };
    let joined = strip_ansi(&render(&s, 100, 0, 48, &ANSI).join("\n"));
    assert!(joined.contains(" 0:") && joined.contains("11:"), "{joined}");
}

#[test]
fn two_columns_when_wide() {
    let procs: Vec<Proc> = (1..=4).map(|i| proc(i, "p", i as f64, 1000)).collect();
    let s = Snapshot { procs, ..snap() };
    let out = render(&s, 80, 0, 48, &ANSI);
    assert_eq!(
        out.iter().map(|l| l.matches("PID").count()).sum::<usize>(),
        2
    );
}

#[test]
fn two_columns_pair_rows() {
    let s = Snapshot {
        procs: vec![proc(1, "left", 1.0, 1000), proc(2, "right", 2.0, 2000)],
        ..snap()
    };
    let out = render(&s, 80, 0, 48, &ANSI);
    assert_eq!(
        out.iter()
            .filter(|l| l.contains("left") || l.contains("right"))
            .count(),
        1
    );
}

#[test]
fn two_columns_odd_count_keeps_all() {
    let procs = vec![
        proc(1, "aa", 1.0, 1),
        proc(2, "bb", 2.0, 2),
        proc(3, "cc", 3.0, 3),
    ];
    let s = Snapshot { procs, ..snap() };
    let all = strip_ansi(&render(&s, 80, 0, 48, &ANSI).join("\n"));
    for name in ["aa", "bb", "cc"] {
        assert!(all.contains(name), "missing {name}");
    }
}

#[test]
fn single_column_when_narrow() {
    let s = Snapshot {
        procs: vec![proc(1, "a", 1.0, 1000), proc(2, "b", 2.0, 2000)],
        ..snap()
    };
    let out = render(&s, 60, 0, 28, &ANSI);
    assert_eq!(
        out.iter().map(|l| l.matches("PID").count()).sum::<usize>(),
        1
    );
}

#[test]
fn single_column_when_one_proc() {
    let s = Snapshot {
        procs: vec![proc(1, "solo", 1.0, 1000)],
        ..snap()
    };
    let out = render(&s, 80, 0, 48, &ANSI);
    assert_eq!(
        out.iter().map(|l| l.matches("PID").count()).sum::<usize>(),
        1
    );
}

#[test]
fn bar_len_has_a_floor() {
    assert_eq!(bar_len_for(0), 4);
    assert_eq!(bar_len_for(36), 4);
    assert_eq!(bar_len_for(80), 48);
}

#[test]
fn core_grid_cell_includes_its_gutter() {
    let g = CoreGrid::new(3, 4, 80, 48, false);
    assert!(g.show_bars);
    assert_eq!(g.cell_w, g.idx_w + 1 + g.bar_len + 4 + 1);
}

#[test]
fn core_grid_drops_bars_when_they_would_be_unreadable() {
    let g = CoreGrid::new(31, 32, 40, 8, false);
    assert!(!g.show_bars);
    assert_eq!(g.cell_w, g.idx_w + 1 + 4 + 1);
}

#[test]
fn core_grid_always_has_at_least_one_column() {
    let g = CoreGrid::new(63, 64, 1, 1, false);
    assert_eq!(g.num_cols, 1);
    assert_eq!(g.rows, 64);
}

#[test]
fn predicted_core_rows_match_rendered_rows() {
    for cores in [2usize, 3, 4, 8, 16, 32, 64, 128] {
        for cols in [40usize, 60, 80, 120, 200] {
            let bar_len = bar_len_for(cols);
            let s = Snapshot {
                cores: (0..cores).map(|i| (i, 10.0, None)).collect(),
                ..snap()
            };
            let drawn = render(&s, cols, 0, bar_len, &ANSI)
                .iter()
                .filter(|l| l.contains(':') && l.contains('%'))
                .count();
            assert_eq!(
                CoreGrid::new(cores - 1, cores, cols, bar_len, false).rows,
                drawn,
                "cores={cores} cols={cols}"
            );
        }
    }
}

#[test]
fn predicted_frame_height_matches_render() {
    for cols in [40usize, 60, 72, 100, 200] {
        for cores in [1usize, 2, 8, 32] {
            for procs in [0usize, 1, 5, 12, 33] {
                let s = full(cores, procs, cols);
                assert_eq!(
                    frame_height(&s, cols, 0),
                    render(&s, cols, 0, bar_len_for(cols), &ANSI).len(),
                    "cols={cols} cores={cores} procs={procs}"
                );
            }
        }
    }
}

/// The prediction has to hold at *every* height, not just the unconstrained
/// one — a panel sized from a prediction one row short of the frame clips its
/// last line. The smallest tier used to report a flat one row while `render`
/// emitted two whenever there was a second row to write the note on.
///
/// The contract is "a snapshot whose process list is within the budget", so
/// the budget is what fills the list here; a longer list is clipped by
/// `render` and the prediction does not claim to describe that.
#[test]
fn predicted_frame_height_matches_render_at_every_height() {
    // From one row up: `lines == 0` means "unconstrained", where the budget
    // is unbounded, and that case is the test above.
    for cols in [0usize, 1, 20, 40, 72, 100, 200] {
        for lines in 1usize..16 {
            for cores in [1usize, 4, 32] {
                let budget = Chrome::of(&full(cores, 0, cols), cols, lines).proc_budget(lines);
                for procs in [0, budget / 2, budget] {
                    let s = full(cores, procs, cols);
                    assert_eq!(
                        frame_height(&s, cols, lines),
                        render(&s, cols, lines, bar_len_for(cols), &ANSI).len(),
                        "cols={cols} lines={lines} cores={cores} procs={procs}"
                    );
                }
            }
        }
    }
}

/// A frame the panel asked for at `lines` rows must not come back taller.
#[test]
fn a_budgeted_frame_never_overflows_the_rows_it_was_given() {
    for cols in [0usize, 1, 20, 40, 72, 100, 200] {
        for lines in 1usize..16 {
            for cores in [1usize, 4, 32] {
                let chrome = Chrome::of(&full(cores, 0, cols), cols, lines);
                let budget = chrome.proc_budget(lines);
                let drawn = render(
                    &full(cores, budget, cols),
                    cols,
                    lines,
                    bar_len_for(cols),
                    &ANSI,
                )
                .len();
                // Below the irreducible chrome height nothing can fit, so the
                // floor is that height rather than `lines`.
                let limit = lines.max(chrome.height());
                assert!(
                    drawn <= limit,
                    "cols={cols} lines={lines} cores={cores}: {drawn} > {limit}"
                );
            }
        }
    }
}

#[test]
fn budgeted_frame_fits_the_panel() {
    for cols in [40usize, 60, 72, 100, 200] {
        for lines in [4usize, 8, 12, 24, 50] {
            for cores in [1usize, 4, 16, 64] {
                let chrome = Chrome::of(&full(cores, 0, cols), cols, lines);
                let budget = chrome.proc_budget(lines);
                let height = frame_height(&full(cores, budget, cols), cols, lines);
                let ctx = format!("cols={cols} lines={lines} cores={cores}");
                if lines >= chrome.height() {
                    assert!(height <= lines, "{ctx}: {height} > {lines}");
                } else {
                    assert_eq!(budget, 0, "{ctx}: asked for processes with no room");
                    assert_eq!(height, chrome.height(), "{ctx}");
                }
            }
        }
    }
}

#[test]
fn budget_fills_the_panel() {
    for cols in [60usize, 80, 120] {
        for lines in [12usize, 24, 40] {
            let chrome = Chrome::of(&full(4, 0, cols), cols, lines);
            let budget = chrome.proc_budget(lines);
            let one_more = frame_height(&full(4, budget + 1, cols), cols, lines);
            assert!(
                one_more > lines,
                "cols={cols} lines={lines}: room for {} more",
                lines - one_more + 1
            );
        }
    }
}

#[test]
fn budget_is_zero_when_there_is_no_room() {
    let chrome = Chrome::of(&full(64, 0, 40), 40, 4);
    assert_eq!(chrome.proc_budget(4), 0);
    assert_eq!(chrome.table_height(0), 0);
}

#[test]
fn wide_panels_budget_two_per_row() {
    let wide_chrome = Chrome::of(&full(1, 0, 100), 100, 20);
    let narrow_chrome = Chrome::of(&full(1, 0, 60), 60, 20);
    assert_eq!(wide_chrome.height(), narrow_chrome.height());
    assert_eq!(
        wide_chrome.proc_budget(20),
        narrow_chrome.proc_budget(20) * 2
    );
}

#[test]
fn empty_snapshot_renders_without_panic() {
    assert!(!render(&Snapshot::default(), 1, 0, 1, &ANSI).is_empty());
}

#[test]
fn tier_adapts_to_lines() {
    let s = full(4, 10, 80);
    assert_eq!(render(&s, 80, 2, 48, &ANSI).len(), 2, "TooSmall");
    assert_eq!(render(&s, 80, 4, 48, &ANSI).len(), 2, "Micro");
    assert_eq!(
        render(&s, 80, 6, 48, &ANSI).len(),
        4,
        "Minimal (Header, CPU, MEM, DSK)"
    );
}
