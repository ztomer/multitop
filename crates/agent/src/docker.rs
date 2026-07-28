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
use std::time::Duration;

use serde_json::Value;

use crate::color::Palette;
use crate::fmt::{center_header, fmt_size, SIZE_MAX, SIZE_PAIR_W};

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
pub fn decode_chunked(body: &[u8]) -> Vec<u8> {
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

pub use crate::docker_cli::*;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Row {
    pub name: String,
    pub status: String,
    pub cpu: String,
    pub cpu_pct: f64,
    pub mem: String,
    pub mem_bytes: u64,
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
                    mem_bytes: s.mem_used,
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
                mem_bytes: 0,
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

pub fn truncate(s: &str, width: usize) -> String {
    if s.chars().count() < width {
        return s.to_string();
    }
    let mut t: String = s.chars().take(width.saturating_sub(3)).collect();
    t.push_str("...");
    t
}

pub fn render(
    host: &str,
    cols: usize,
    max_rows: usize,
    rows: &[Row],
    pal: &Palette,
    sort_by: crate::SortBy,
) -> Vec<String> {
    let mut out = Vec::with_capacity(rows.len() + 4);
    out.push(center_header(host, cols, pal));

    if rows.is_empty() {
        out.push(format!(" {}No running containers{}", pal.gray, pal.reset));
        return out;
    }

    let mut sorted_rows = rows.to_vec();
    match sort_by {
        crate::SortBy::Cpu => sorted_rows.sort_unstable_by(|a, b| {
            b.cpu_pct
                .partial_cmp(&a.cpu_pct)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| b.mem_bytes.cmp(&a.mem_bytes))
                .then_with(|| a.name.cmp(&b.name))
        }),
        crate::SortBy::Mem => sorted_rows.sort_unstable_by(|a, b| {
            b.mem_bytes
                .cmp(&a.mem_bytes)
                .then_with(|| b.cpu_pct.partial_cmp(&a.cpu_pct).unwrap_or(std::cmp::Ordering::Equal))
                .then_with(|| a.name.cmp(&b.name))
        }),
    }

    out.push(format!(
        " {}{:<NAME_W$}  {:<STATUS_W$}  {:<CPU_W$}  {:<MEM_W$}{}",
        pal.bold, "NAME", "STATUS", "CPU", "MEM", pal.reset,
    ));
    let rule = format!(
        " {}{}{}",
        pal.gray,
        "\u{2500}".repeat(cols.saturating_sub(2)),
        pal.reset
    );
    out.push(rule.clone());

    // Budget: header(1) + col-header(1) + rule(1) + bottom-rule(1) = 4 rows of
    // chrome.  Everything else is container rows.  When the panel is too short,
    // show as many as fit and a summary.
    let chrome_rows = 4;
    let body_budget = max_rows.saturating_sub(chrome_rows);
    let (visible, overflow) = if body_budget >= sorted_rows.len() || max_rows == 0 {
        (&sorted_rows[..], 0)
    } else {
        // Reserve one row for the "…+N more" line.
        let show = body_budget.saturating_sub(1);
        (&sorted_rows[..show], sorted_rows.len() - show)
    };

    for r in visible {
        let cpu_c = if r.cpu_pct >= 50.0 {
            pal.yellow
        } else {
            pal.green
        };
        out.push(format!(
            " {}{:<NAME_W$}{}  {}{:<STATUS_W$}{}  {}{:<CPU_W$}{}  {}{:<MEM_W$}{}",
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

    if overflow > 0 {
        out.push(format!(
            " {}\u{2026}+{} more{}",
            pal.gray, overflow, pal.reset
        ));
    }

    out.push(rule);
    out
}
