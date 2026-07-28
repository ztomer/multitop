//! Golden-file parity tests for the renderer.
//!
//! The host header, CPU block and MEM/DSK/NET rows must render byte-for-byte
//! identically to the committed golden file — those rows are the panel's
//! visual contract.
//!
//! The golden file (`tests/parity/golden.txt`) carries its own inputs so the
//! case list exists in one place only. The process table is intentionally
//! *not* covered: its MEM column is wider, and an empty table draws nothing.
//! Those behaviours are pinned by unit tests in `render.rs`.

use multitop_agent::color::ANSI;
use multitop_agent::proc::Usage;
use multitop_agent::render::{bar_len_for, render, Snapshot};

const GOLDEN: &str = include_str!("../../../tests/parity/golden.txt");

struct Case {
    name: String,
    cols: usize,
    snapshot: Snapshot,
    expected: Vec<String>,
}

fn usage(spec: &str) -> Usage {
    let f: Vec<&str> = spec.split(',').collect();
    Usage {
        total: f[0].parse().expect("total"),
        used: f[1].parse().expect("used"),
        pct: f[2].parse().expect("pct"),
    }
}

fn cores(spec: &str) -> Vec<(usize, f64)> {
    if spec.is_empty() {
        return Vec::new();
    }
    spec.split(',')
        .map(|c| {
            let (i, p) = c.split_once(':').expect("core spec");
            (i.parse().expect("core index"), p.parse().expect("core pct"))
        })
        .collect()
}

fn parse_golden(text: &str) -> Vec<Case> {
    let mut cases: Vec<Case> = Vec::new();
    for line in text.lines() {
        let Some(spec) = line.strip_prefix("=== ") else {
            if let Some(case) = cases.last_mut() {
                case.expected.push(line.to_string());
            }
            continue;
        };
        let f: Vec<&str> = spec.split('\t').collect();
        assert_eq!(f.len(), 9, "malformed case header: {spec:?}");
        cases.push(Case {
            name: f[0].to_string(),
            cols: f[1].parse().expect("cols"),
            snapshot: Snapshot {
                host: f[8].to_string(),
                cpu_pct: f[2].parse().expect("cpu"),
                cores: cores(f[3]),
                mem: usage(f[4]),
                disk: usage(f[5]),
                rx_rate: f[6].parse().expect("rx"),
                tx_rate: f[7].parse().expect("tx"),
                procs: Vec::new(),
            },
            expected: Vec::new(),
        });
    }
    cases
}

/// Render an invisible character visibly, so a diff shows what differs.
fn escape(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '\x1b' => "\\e".to_string(),
            c if (c as u32) < 0x20 => format!("\\x{:02x}", c as u32),
            c => c.to_string(),
        })
        .collect()
}

#[test]
fn golden_file_is_populated() {
    let cases = parse_golden(GOLDEN);
    assert!(cases.len() >= 60, "only {} cases", cases.len());
    assert!(
        cases.iter().all(|c| !c.expected.is_empty()),
        "a case has no output"
    );
}

#[test]
fn chrome_rows_match_golden_output() {
    let cases = parse_golden(GOLDEN);
    let mut failures: Vec<String> = Vec::new();

    for case in &cases {
        let got = render(&case.snapshot, case.cols, bar_len_for(case.cols), &ANSI);

        if got.len() != case.expected.len() {
            failures.push(format!(
                "{}: {} rows, expected {}",
                case.name,
                got.len(),
                case.expected.len()
            ));
            continue;
        }
        for (i, (g, e)) in got.iter().zip(&case.expected).enumerate() {
            if g != e {
                failures.push(format!(
                    "{} line {i}\n     got: {}\n  wanted: {}",
                    case.name,
                    escape(g),
                    escape(e)
                ));
            }
        }
    }

    assert!(
        failures.is_empty(),
        "{} of {} cases differ from golden output:\n{}",
        failures.len(),
        cases.len(),
        failures.join("\n")
    );
}
