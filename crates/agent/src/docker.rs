//! Docker view.
//!
//! Container stats come from the daemon socket rather than `docker stats
//! --no-stream`. The CLI form is slow by construction: it asks the daemon for
//! a *streaming* stats feed and waits for the second sample, which costs a
//! full second per invocation regardless of how fast the host is. Two
//! `one-shot` reads with a window we choose gets the same numbers in a
//! fraction of the time, and samples every container concurrently.
//!
//! The `docker` CLI remains as a fallback for hosts where the socket is not
//! readable by the login user.

use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::process::Command;
use std::time::Duration;

use serde_json::Value;

use crate::color::Palette;
use crate::fmt::{fmt_size, fullwidth, fullwidth_display_width, SIZE_MAX, SIZE_PAIR_W};

/// Window between the two CPU samples. Long enough for the counters to move,
/// short enough that pressing `d` feels instant.
const SAMPLE_WINDOW: Duration = Duration::from_millis(250);
const IO_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_WORKERS: usize = 8;
/// Pinned so a newer daemon cannot change the response shape underneath us.
const API: &str = "/v1.41";

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Container {
    pub id: String,
    pub name: String,
    pub status: String,
    pub image: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Stats {
    pub cpu_pct: f64,
    pub mem_used: u64,
    pub mem_limit: u64,
}

impl Stats {
    pub fn mem_string(&self) -> String {
        format!("{}/{}", fmt_size(self.mem_used), fmt_size(self.mem_limit))
    }
}

// ---------------------------------------------------------------- transport

fn socket_path() -> String {
    match std::env::var("DOCKER_HOST") {
        Ok(h) => h
            .strip_prefix("unix://")
            .unwrap_or("/var/run/docker.sock")
            .to_string(),
        Err(_) => "/var/run/docker.sock".to_string(),
    }
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Decode `Transfer-Encoding: chunked` bodies.
fn decode_chunked(body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(body.len());
    let mut pos = 0;
    while pos < body.len() {
        let Some(eol) = find_subslice(&body[pos..], b"\r\n") else {
            break;
        };
        let header = &body[pos..pos + eol];
        let size_txt = std::str::from_utf8(header)
            .unwrap_or("")
            .split(';')
            .next()
            .unwrap_or("");
        let Ok(size) = usize::from_str_radix(size_txt.trim(), 16) else {
            break;
        };
        pos += eol + 2;
        if size == 0 {
            break;
        }
        let end = (pos + size).min(body.len());
        out.extend_from_slice(&body[pos..end]);
        pos = end + 2; // trailing CRLF
    }
    out
}

/// Minimal HTTP/1.1 GET over a unix socket.
///
/// `Connection: close` lets us read to EOF instead of tracking content
/// lengths, and the daemon answers every one of these in a single response.
fn http_get(path: &str) -> io::Result<Vec<u8>> {
    let mut stream = UnixStream::connect(socket_path())?;
    stream.set_read_timeout(Some(IO_TIMEOUT))?;
    stream.set_write_timeout(Some(IO_TIMEOUT))?;
    let req = format!(
        "GET {path} HTTP/1.1\r\nHost: localhost\r\nAccept: application/json\r\nConnection: close\r\n\r\n"
    );
    stream.write_all(req.as_bytes())?;
    stream.flush()?;

    let mut raw = Vec::with_capacity(8192);
    stream.read_to_end(&mut raw)?;

    let split = find_subslice(&raw, b"\r\n\r\n")
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "no header terminator"))?;
    let head = String::from_utf8_lossy(&raw[..split]).to_ascii_lowercase();
    let body = &raw[split + 4..];

    let ok = head
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|c| c.parse::<u16>().ok())
        .is_some_and(|c| (200..300).contains(&c));
    if !ok {
        return Err(io::Error::other(format!(
            "docker api returned {}",
            head.lines().next().unwrap_or("?")
        )));
    }

    if head.contains("transfer-encoding: chunked") {
        Ok(decode_chunked(body))
    } else {
        Ok(body.to_vec())
    }
}

// ------------------------------------------------------------------ parsing

/// Extract the container list from `GET /containers/json`.
pub fn parse_container_list(json: &str) -> Vec<Container> {
    let Ok(Value::Array(items)) = serde_json::from_str::<Value>(json) else {
        return Vec::new();
    };
    items
        .iter()
        .map(|c| Container {
            id: c["Id"].as_str().unwrap_or("").to_string(),
            // Names are API paths ("/web"); the CLI shows them without the slash.
            name: c["Names"][0]
                .as_str()
                .unwrap_or("")
                .trim_start_matches('/')
                .to_string(),
            status: c["Status"].as_str().unwrap_or("").to_string(),
            image: c["Image"].as_str().unwrap_or("").to_string(),
        })
        .filter(|c| !c.id.is_empty())
        .collect()
}

/// CPU/memory counters from one `/stats?one-shot=true` response.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct StatSample {
    pub cpu_total: u64,
    pub system_total: u64,
    pub online_cpus: u64,
    pub mem_used: u64,
    pub mem_limit: u64,
}

pub fn parse_stat_sample(json: &str) -> Option<StatSample> {
    let v: Value = serde_json::from_str(json).ok()?;
    let cpu = &v["cpu_stats"];
    let mem = &v["memory_stats"];

    // online_cpus is absent on older daemons; percpu_usage length is the
    // documented fallback, and 1 keeps the maths finite if both are missing.
    let online_cpus = cpu["online_cpus"]
        .as_u64()
        .or_else(|| {
            cpu["cpu_usage"]["percpu_usage"]
                .as_array()
                .map(|a| a.len() as u64)
        })
        .unwrap_or(1)
        .max(1);

    // Docker subtracts page cache to match what `docker stats` reports.
    let usage = mem["usage"].as_u64().unwrap_or(0);
    let inactive = mem["stats"]["inactive_file"]
        .as_u64()
        .or_else(|| mem["stats"]["total_inactive_file"].as_u64())
        .unwrap_or(0);

    // An unconstrained container reports a near-`u64::MAX` limit. That is not
    // a number worth printing, and it would overflow the memory column, so
    // treat it as "no limit" like the CLI does.
    let limit = mem["limit"].as_u64().unwrap_or(0);
    let mem_limit = if limit >= SIZE_MAX { 0 } else { limit };

    Some(StatSample {
        cpu_total: cpu["cpu_usage"]["total_usage"].as_u64().unwrap_or(0),
        system_total: cpu["system_cpu_usage"].as_u64().unwrap_or(0),
        online_cpus,
        mem_used: usage.saturating_sub(inactive),
        mem_limit,
    })
}

/// CPU percentage between two samples, using Docker's own formula.
pub fn cpu_pct_between(prev: &StatSample, curr: &StatSample) -> f64 {
    let cpu_delta = curr.cpu_total.saturating_sub(prev.cpu_total) as f64;
    let sys_delta = curr.system_total.saturating_sub(prev.system_total) as f64;
    if sys_delta <= 0.0 || cpu_delta <= 0.0 {
        return 0.0;
    }
    cpu_delta / sys_delta * curr.online_cpus as f64 * 100.0
}

// ---------------------------------------------------------------- collection

fn fetch_sample(id: &str) -> Option<StatSample> {
    let body = http_get(&format!(
        "{API}/containers/{id}/stats?stream=false&one-shot=true"
    ))
    .ok()?;
    parse_stat_sample(&String::from_utf8_lossy(&body))
}

/// Sample every container over one shared window, `MAX_WORKERS` at a time.
fn collect_stats_via_socket(containers: &[Container]) -> HashMap<String, Stats> {
    let ids: Vec<&str> = containers.iter().map(|c| c.id.as_str()).collect();
    if ids.is_empty() {
        return HashMap::new();
    }

    let sample_all = |ids: &[&str]| -> Vec<Option<StatSample>> {
        let chunk = ids.len().div_ceil(MAX_WORKERS).max(1);
        std::thread::scope(|scope| {
            let handles: Vec<_> = ids
                .chunks(chunk)
                .map(|group| {
                    scope.spawn(move || group.iter().map(|id| fetch_sample(id)).collect::<Vec<_>>())
                })
                .collect();
            handles
                .into_iter()
                .flat_map(|h| h.join().unwrap_or_default())
                .collect()
        })
    };

    let first = sample_all(&ids);
    std::thread::sleep(SAMPLE_WINDOW);
    let second = sample_all(&ids);

    ids.iter()
        .zip(first)
        .zip(second)
        .filter_map(|((id, a), b)| {
            let (a, b) = (a?, b?);
            Some((
                id.to_string(),
                Stats {
                    cpu_pct: cpu_pct_between(&a, &b),
                    mem_used: b.mem_used,
                    mem_limit: b.mem_limit,
                },
            ))
        })
        .collect()
}

// -------------------------------------------------------------- CLI fallback

fn docker_cli(args: &[&str]) -> Option<String> {
    let out = Command::new("docker").args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8(out.stdout).ok()
}

pub fn parse_cli_ps(text: &str) -> Vec<Container> {
    text.lines()
        .filter_map(|line| {
            let f: Vec<&str> = line.split('\t').collect();
            if f.len() < 3 || f[0].is_empty() {
                return None;
            }
            Some(Container {
                name: f[0].to_string(),
                status: f[1].to_string(),
                image: f[2].to_string(),
                id: f.get(3).unwrap_or(&"").to_string(),
            })
        })
        .collect()
}

/// `docker stats` prints preformatted strings; keep them as-is rather than
/// round-tripping through a parse.
pub fn parse_cli_stats(text: &str) -> HashMap<String, (String, String)> {
    text.lines()
        .filter_map(|line| {
            let f: Vec<&str> = line.split('\t').collect();
            if f.len() < 3 {
                return None;
            }
            Some((f[0].to_string(), (f[1].to_string(), f[2].to_string())))
        })
        .collect()
}

/// One container as the table will draw it, however it was collected.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Row {
    pub name: String,
    pub status: String,
    pub cpu: String,
    pub cpu_pct: f64,
    pub mem: String,
}

/// Gather rows, preferring the socket and falling back to the CLI.
pub fn collect() -> Vec<Row> {
    if let Ok(body) = http_get(&format!("{API}/containers/json")) {
        let containers = parse_container_list(&String::from_utf8_lossy(&body));
        let stats = collect_stats_via_socket(&containers);
        return containers
            .into_iter()
            .map(|c| {
                let s = stats.get(&c.id).copied().unwrap_or_default();
                Row {
                    name: c.name,
                    status: c.status,
                    cpu: format!("{:.1}%", s.cpu_pct),
                    cpu_pct: s.cpu_pct,
                    mem: if s.mem_limit > 0 {
                        s.mem_string()
                    } else {
                        "-".into()
                    },
                }
            })
            .collect();
    }

    let Some(ps) = docker_cli(&[
        "ps",
        "--format",
        "{{.Names}}\t{{.Status}}\t{{.Image}}\t{{.ID}}",
    ]) else {
        return Vec::new();
    };
    let containers = parse_cli_ps(&ps);
    let stats = docker_cli(&[
        "stats",
        "--no-stream",
        "--format",
        "{{.Name}}\t{{.CPUPerc}}\t{{.MemUsage}}",
    ])
    .map(|s| parse_cli_stats(&s))
    .unwrap_or_default();

    containers
        .into_iter()
        .map(|c| {
            let (cpu, mem) = stats
                .get(&c.name)
                .cloned()
                .unwrap_or_else(|| ("0.0%".into(), "0B / 0B".into()));
            Row {
                cpu_pct: cpu.trim_end_matches('%').parse().unwrap_or(0.0),
                name: c.name,
                status: c.status,
                cpu,
                mem,
            }
        })
        .collect()
}

// ----------------------------------------------------------------- rendering

const NAME_W: usize = 20;
const STATUS_W: usize = 16;
/// Wide enough for `999.9%` on a single core and `6400.0%` on a big host.
const CPU_W: usize = 7;
/// A `used/total` pair, sized from the formatter that produces it.
const MEM_W: usize = SIZE_PAIR_W;

fn truncate(s: &str, width: usize) -> String {
    if s.chars().count() < width {
        return s.to_string();
    }
    let mut t: String = s.chars().take(width.saturating_sub(3)).collect();
    t.push_str("...");
    t
}

pub fn render(host: &str, cols: usize, rows: &[Row], pal: &Palette) -> Vec<String> {
    let mut out = Vec::with_capacity(rows.len() + 4);
    let disp_w = fullwidth_display_width(host);
    out.push(format!(
        "{}{}{}{}  {}{}{}",
        pal.cyan,
        pal.bold,
        fullwidth(host),
        pal.reset,
        pal.gray,
        "\u{2500}".repeat(cols.saturating_sub(disp_w).saturating_sub(6)),
        pal.reset,
    ));

    if rows.is_empty() {
        out.push(format!(" {}No running containers{}", pal.gray, pal.reset));
        return out;
    }

    out.push(format!(
        " {}{:<NAME_W$}  {:<STATUS_W$}  {:>CPU_W$}  {:>MEM_W$}{}",
        pal.bold, "NAME", "STATUS", "CPU", "MEM", pal.reset,
    ));
    let rule = format!(
        " {}{}{}",
        pal.gray,
        "\u{2500}".repeat(cols.saturating_sub(2)),
        pal.reset
    );
    out.push(rule.clone());

    for r in rows {
        let cpu_c = if r.cpu_pct >= 50.0 {
            pal.yellow
        } else {
            pal.green
        };
        out.push(format!(
            " {}{:<NAME_W$}{}  {}{:<STATUS_W$}{}  {}{:>CPU_W$}{}  {}{:>MEM_W$}{}",
            pal.white,
            truncate(&r.name, NAME_W),
            pal.reset,
            pal.status_color(&r.status),
            truncate(&r.status, STATUS_W),
            pal.reset,
            cpu_c,
            r.cpu,
            pal.reset,
            pal.cyan,
            r.mem,
            pal.reset,
        ));
    }

    out.push(rule);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::{strip_ansi, ANSI};

    fn row(name: &str, status: &str, cpu: f64) -> Row {
        Row {
            name: name.into(),
            status: status.into(),
            cpu: format!("{cpu:.1}%"),
            cpu_pct: cpu,
            mem: "1.0MiB/1.0GiB".into(),
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
        // Missing Id means we cannot key stats to it, so it is dropped.
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

    /// A container with no memory limit reports a sentinel near u64::MAX;
    /// printing it would both be meaningless and overflow the column.
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
        // 100/1000 * 4 * 100 = 40%
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

    /// A restarted container resets its counters; the result must not go
    /// negative or NaN.
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
        // A truncated stream yields what arrived rather than looping.
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

    #[test]
    fn render_empty_says_so() {
        let out = render("h", 80, &[], &ANSI);
        assert_eq!(out.len(), 2);
        assert!(out[1].contains("No running containers"));
    }

    #[test]
    fn render_host_header_is_fullwidth() {
        assert!(render("h", 80, &[], &ANSI)[0].contains('\u{ff48}'));
    }

    #[test]
    fn render_lists_containers() {
        let rows = vec![row("web", "Up 3 hours", 5.0), row("db", "Exited (0)", 70.0)];
        let out = render("h", 80, &rows, &ANSI);
        let all = out.join("\n");
        assert!(all.contains("web"));
        assert!(all.contains("db"));
        assert!(all.contains("NAME"));
        assert!(all.contains("STATUS"));
    }

    #[test]
    fn render_colors_by_status() {
        let rows = vec![row("up", "Up 1 hour", 1.0), row("gone", "Exited (0)", 1.0)];
        let out = render("h", 80, &rows, &ANSI);
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
        let out = render("h", 80, &[row("busy", "Up", 75.0)], &ANSI);
        assert!(out
            .iter()
            .find(|l| l.contains("busy"))
            .unwrap()
            .contains(ANSI.yellow));
        let out = render("h", 80, &[row("calm", "Up", 5.0)], &ANSI);
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
        let out = strip_ansi(&render("h", 80, &rows, &ANSI).join("\n"));
        assert!(out.contains("..."));
    }

    /// Every row of the table, header included, must be exactly as wide as
    /// every other. This is the regression that shipped: the MEM column was
    /// 12 wide while `used/total` reaches 19, so a container with a
    /// three-digit memory figure pushed its own row out of line.
    #[test]
    fn render_rows_are_aligned() {
        let rows = vec![
            row("a", "Up 1 second", 1.0),
            row("a-much-longer-name", "Exited (137) 4 weeks ago", 99.0),
            Row {
                name: "firefly-db".into(),
                status: "Up 2 hours (healthy)".into(),
                cpu: "1.8%".into(),
                cpu_pct: 1.8,
                mem: Stats {
                    cpu_pct: 0.0,
                    mem_used: 29 * 1024 * 1024,
                    mem_limit: 26_700_000_000,
                }
                .mem_string(),
            },
            Row {
                name: "jellyfin".into(),
                status: "Up 2 hours (healthy)".into(),
                cpu: "640.0%".into(),
                cpu_pct: 640.0,
                // The widest pair `fmt_size` can produce on both sides.
                mem: Stats {
                    cpu_pct: 0.0,
                    mem_used: 1023 * 1024,
                    mem_limit: 1023 * (1 << 30),
                }
                .mem_string(),
            },
            Row {
                name: "no-limit".into(),
                status: "Up".into(),
                cpu: "0.0%".into(),
                cpu_pct: 0.0,
                mem: "-".into(),
            },
        ];

        for cols in [80usize, 100, 118, 200] {
            let out = render("h", cols, &rows, &ANSI);
            // Header row through the last container row.
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
        let out = render("h", 4, &[row("x", "Up", 1.0)], &ANSI);
        assert!(!out.is_empty());
    }
}
