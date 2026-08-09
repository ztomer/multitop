use multitop_agent::color::{strip_ansi, ANSI};
use multitop_agent::docker::*;
use multitop_agent::fmt::SIZE_MAX;

fn row(name: &str, status: &str, cpu: f64) -> Row {
    Row {
        name: name.into(),
        status: status.into(),
        image: "nginx:latest".into(),
        cpu: format!("{cpu:.1}%"),
        cpu_pct: cpu,
        mem: "1.0MiB/1.0GiB".into(),
        mem_bytes: 1024 * 1024,
    }
}

#[test]
fn container_list_parsed() {
    let json = r#"[
        {"Id":"abc123","Names":["/web"],"Status":"Up 3 hours","Image":"nginx"},
        {"Id":"def456","Names":["/db"],"Status":"Up 2 days","Image":"postgres"}
    ]"#;
    let c = parse_container_list(json);
    assert_eq!(c.len(), 2);
    assert_eq!(c[0].name, "web", "leading slash must be stripped");
    assert_eq!(c[0].id, "abc123");
    assert_eq!(c[1].status, "Up 2 days");
}

#[test]
fn container_list_tolerates_garbage() {
    assert!(parse_container_list("").is_empty());
    assert!(parse_container_list("not json").is_empty());
    assert!(parse_container_list("{}").is_empty());
    assert!(parse_container_list("[]").is_empty());
    assert!(parse_container_list(r#"[{"Names":["/x"]}]"#).is_empty());
}

#[test]
fn container_list_handles_missing_names() {
    let c = parse_container_list(r#"[{"Id":"a1"}]"#);
    assert_eq!(c.len(), 1);
    assert_eq!(c[0].name, "");
}

#[test]
fn stat_sample_parsed() {
    let json = r#"{
        "cpu_stats":{"cpu_usage":{"total_usage":1000},"system_cpu_usage":100000,"online_cpus":4},
        "memory_stats":{"usage":2000,"limit":8000,"stats":{"inactive_file":500}}
    }"#;
    let s = parse_stat_sample(json).unwrap();
    assert_eq!(s.cpu_total, 1000);
    assert_eq!(s.system_total, 100_000);
    assert_eq!(s.online_cpus, 4);
    assert_eq!(s.mem_used, 1500, "page cache is excluded");
    assert_eq!(s.mem_limit, 8000);
}

#[test]
fn stat_sample_falls_back_to_percpu_length() {
    let json = r#"{
        "cpu_stats":{"cpu_usage":{"total_usage":1,"percpu_usage":[1,2,3]},"system_cpu_usage":2},
        "memory_stats":{"usage":10,"limit":20}
    }"#;
    assert_eq!(parse_stat_sample(json).unwrap().online_cpus, 3);
}

#[test]
fn stat_sample_online_cpus_never_zero() {
    let json = r#"{"cpu_stats":{"online_cpus":0},"memory_stats":{}}"#;
    assert_eq!(parse_stat_sample(json).unwrap().online_cpus, 1);
}

#[test]
fn stat_sample_accepts_cgroup_v1_field_name() {
    let json =
        r#"{"cpu_stats":{},"memory_stats":{"usage":900,"stats":{"total_inactive_file":400}}}"#;
    assert_eq!(parse_stat_sample(json).unwrap().mem_used, 500);
}

#[test]
fn stat_sample_treats_unlimited_memory_as_no_limit() {
    let json = r#"{"cpu_stats":{},"memory_stats":{"usage":100,"limit":9223372036854771712}}"#;
    assert_eq!(parse_stat_sample(json).unwrap().mem_limit, 0);

    let real = format!(
        r#"{{"cpu_stats":{{}},"memory_stats":{{"usage":100,"limit":{}}}}}"#,
        SIZE_MAX - 1
    );
    assert_eq!(parse_stat_sample(&real).unwrap().mem_limit, SIZE_MAX - 1);
}

#[test]
fn stat_sample_rejects_garbage() {
    assert!(parse_stat_sample("nope").is_none());
}

#[test]
fn cpu_pct_uses_docker_formula() {
    let prev = StatSample {
        cpu_total: 100,
        system_total: 1000,
        online_cpus: 4,
        ..Default::default()
    };
    let curr = StatSample {
        cpu_total: 200,
        system_total: 2000,
        online_cpus: 4,
        ..Default::default()
    };
    assert!((cpu_pct_between(&prev, &curr) - 40.0).abs() < 1e-9);
}

#[test]
fn cpu_pct_is_zero_without_movement() {
    let s = StatSample {
        cpu_total: 100,
        system_total: 1000,
        online_cpus: 2,
        ..Default::default()
    };
    assert_eq!(cpu_pct_between(&s, &s), 0.0);
}

#[test]
fn cpu_pct_survives_counter_reset() {
    let prev = StatSample {
        cpu_total: 5000,
        system_total: 9000,
        online_cpus: 2,
        ..Default::default()
    };
    let curr = StatSample {
        cpu_total: 10,
        system_total: 20,
        online_cpus: 2,
        ..Default::default()
    };
    let pct = cpu_pct_between(&prev, &curr);
    assert!(pct.is_finite() && pct >= 0.0);
}

#[test]
fn chunked_body_decoded() {
    let body = b"5\r\nhello\r\n6\r\n world\r\n0\r\n\r\n";
    assert_eq!(
        decode_chunked(body),
        b" world"
            .iter()
            .copied()
            .fold(b"hello".to_vec(), |mut v, c| {
                v.push(c);
                v
            })
    );
}

#[test]
fn chunked_body_handles_extensions_and_truncation() {
    assert_eq!(decode_chunked(b"3;x=1\r\nabc\r\n0\r\n\r\n"), b"abc");
    assert_eq!(decode_chunked(b"9\r\nabc"), b"abc");
    assert!(decode_chunked(b"").is_empty());
    assert!(decode_chunked(b"zz\r\n").is_empty());
}

#[test]
fn cli_ps_parsed() {
    let text = "web\tUp 3 hours\tnginx\tabc123\ndb\tUp 2 days\tpostgres\tdef456\n";
    let c = parse_cli_ps(text);
    assert_eq!(c.len(), 2);
    assert_eq!(c[0].name, "web");
    assert_eq!(c[1].image, "postgres");
}

#[test]
fn cli_ps_skips_short_rows() {
    assert!(parse_cli_ps("").is_empty());
    assert!(parse_cli_ps("only\tone\n").is_empty());
    assert!(parse_cli_ps("\ta\tb\n").is_empty());
}

#[test]
fn cli_stats_parsed() {
    let m = parse_cli_stats("web\t12.34%\t100MiB / 1GiB\n");
    assert_eq!(m["web"].0, "12.34%");
    assert_eq!(m["web"].1, "100MiB / 1GiB");
}

#[test]
fn cli_stats_skips_short_rows() {
    assert!(parse_cli_stats("web\t1%\n").is_empty());
}

use multitop_agent::SortBy;

#[test]
fn render_empty_says_so() {
    let out = render("h", 80, 0, &[], &ANSI, SortBy::Cpu);
    assert_eq!(out.len(), 2);
    assert!(out[1].contains("No running containers"));
}

#[test]
fn render_host_header_is_fullwidth() {
    assert!(render("h", 80, 0, &[], &ANSI, SortBy::Cpu)[0].contains('\u{ff48}'));
}

#[test]
fn render_lists_containers() {
    let rows = vec![row("web", "Up 3 hours", 5.0), row("db", "Exited (0)", 70.0)];
    let out = render("h", 80, 0, &rows, &ANSI, SortBy::Cpu);
    let all = out.join("\n");
    assert!(all.contains("web"));
    assert!(all.contains("db"));
    assert!(all.contains("NAME"));
    assert!(all.contains("STATUS"));
}

#[test]
fn render_colors_by_status() {
    let rows = vec![row("up", "Up 1 hour", 1.0), row("gone", "Exited (0)", 1.0)];
    let out = render("h", 80, 0, &rows, &ANSI, SortBy::Cpu);
    assert!(out
        .iter()
        .find(|l| l.contains("up "))
        .unwrap()
        .contains(ANSI.green));
    assert!(out
        .iter()
        .find(|l| l.contains("gone"))
        .unwrap()
        .contains(ANSI.yellow));
}

#[test]
fn render_flags_busy_containers() {
    let out = render("h", 80, 0, &[row("busy", "Up", 75.0)], &ANSI, SortBy::Cpu);
    assert!(out
        .iter()
        .find(|l| l.contains("busy"))
        .unwrap()
        .contains(ANSI.yellow));
    let out = render("h", 80, 0, &[row("calm", "Up", 5.0)], &ANSI, SortBy::Cpu);
    assert!(out
        .iter()
        .find(|l| l.contains("calm"))
        .unwrap()
        .contains(ANSI.green));
}

#[test]
fn render_truncates_long_fields() {
    let rows = vec![row(
        "a-really-long-container-name",
        "Up 3 hours and counting",
        1.0,
    )];
    let out = strip_ansi(&render("h", 80, 0, &rows, &ANSI, SortBy::Cpu).join("\n"));
    assert!(out.contains("..."));
}

#[test]
fn render_rows_are_aligned() {
    let rows = vec![
        row("a", "Up 1 second", 1.0),
        row("a-much-longer-name", "Exited (137) 4 weeks ago", 99.0),
        Row {
            name: "firefly-db".into(),
            status: "Up 2 hours (healthy)".into(),
            image: "nginx:latest".into(),
            cpu: "1.8%".into(),
            cpu_pct: 1.8,
            mem: Stats {
                cpu_pct: 0.0,
                mem_used: 29 * 1024 * 1024,
                mem_limit: 26_700_000_000,
            }
            .mem_string(),
            mem_bytes: 29 * 1024 * 1024,
        },
        Row {
            name: "jellyfin".into(),
            status: "Up 2 hours (healthy)".into(),
            image: "nginx:latest".into(),
            cpu: "640.0%".into(),
            cpu_pct: 640.0,
            mem: Stats {
                cpu_pct: 0.0,
                mem_used: 1023 * 1024,
                mem_limit: 1023 * (1 << 30),
            }
            .mem_string(),
            mem_bytes: 1023 * 1024,
        },
        Row {
            name: "no-limit".into(),
            status: "Up".into(),
            image: "nginx:latest".into(),
            cpu: "0.0%".into(),
            cpu_pct: 0.0,
            mem: "-".into(),
            mem_bytes: 0,
        },
    ];

    for cols in [80usize, 100, 118, 200] {
        let out = render("h", cols, 0, &rows, &ANSI, SortBy::Cpu);
        let widths: Vec<usize> = out[1..out.len() - 1]
            .iter()
            .filter(|l| !l.contains('\u{2500}'))
            .map(|l| strip_ansi(l).chars().count())
            .collect();
        assert_eq!(widths.len(), rows.len() + 1, "header plus every row");
        assert!(
            widths.windows(2).all(|w| w[0] == w[1]),
            "cols={cols} widths={widths:?}\n{}",
            out.iter()
                .map(|l| strip_ansi(l))
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
}

#[test]
fn truncate_is_char_safe() {
    assert_eq!(
        truncate("\u{4f60}\u{597d}\u{4e16}\u{754c}", 4),
        "\u{4f60}..."
    );
    assert_eq!(truncate("ab", 4), "ab");
}

#[test]
fn mem_string_formats_both_sides() {
    let s = Stats {
        cpu_pct: 0.0,
        mem_used: 1024 * 1024,
        mem_limit: 1024 * 1024 * 1024,
    };
    assert_eq!(s.mem_string(), "1.0MiB/1.0GiB");
}

#[test]
fn render_survives_narrow_panel() {
    let out = render("h", 4, 0, &[row("x", "Up", 1.0)], &ANSI, SortBy::Cpu);
    assert!(!out.is_empty());
}

#[test]
fn render_respects_max_rows_budget() {
    let rows: Vec<_> = (0..10).map(|i| row(&format!("c{i}"), "Up", 1.0)).collect();
    let out = render("h", 80, 6, &rows, &ANSI, SortBy::Cpu);
    assert_eq!(out.len(), 6);
    assert!(out
        .iter()
        .any(|l| l.contains("…+9 more") || l.contains("...+9 more")));
}
