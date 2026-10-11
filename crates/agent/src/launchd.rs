//! launchd as a cron table, for `multitop-agent mcp` on a Mac.
//!
//! A Mac has no cron that anyone uses; its scheduled jobs are the user's
//! launch agents. `launchctl list` gives, per loaded job, the PID while it
//! runs and the last exit status -- which is what the Ops view's cron
//! section shows for a Linux host. Only the user's own agents
//! (`~/Library/LaunchAgents/*.plist`) are jobs here: the user domain lists
//! some 570 labels, nearly all Apple's, and a cron section that showed them
//! would bury the dozen the user wrote. A plist that is not loaded is not
//! scheduled, so it is not listed -- the same as a commented-out crontab line.
//!
//! Parsing is pure and tested here; what runs `launchctl`, `plutil` and `ps`
//! is [`crate::mcp_tools::Live`].

use std::collections::HashMap;

use serde_json::{json, Value};

/// One row of `launchctl list`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    /// Set while the job runs.
    pub pid: Option<u32>,
    /// The last exit status; negative is the signal that ended it.
    pub status: i64,
    pub label: String,
}

/// `launchctl list`: `PID\tStatus\tLabel`, `-` for a job not running.
#[must_use]
pub fn parse_list(text: &str) -> Vec<Row> {
    text.lines()
        .filter_map(|line| {
            let mut f = line.split('\t');
            let (pid, status, label) = (f.next()?, f.next()?, f.next()?);
            Some(Row {
                pid: pid.trim().parse().ok(),
                status: status.trim().parse().ok()?,
                label: label.trim().to_string(),
            })
        })
        .filter(|r| !r.label.is_empty())
        .collect()
}

/// `ps -o etime`: `[[dd-]hh:]mm:ss`, as seconds.
#[must_use]
pub fn parse_etime(s: &str) -> Option<i64> {
    let s = s.trim();
    let (days, clock) = match s.split_once('-') {
        Some((d, rest)) => (d.parse::<i64>().ok()?, rest),
        None => (0, s),
    };
    let parts: Vec<i64> = clock
        .split(':')
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()?;
    let (h, m, sec) = match parts.as_slice() {
        [m, s] => (0, *m, *s),
        [h, m, s] => (*h, *m, *s),
        _ => return None,
    };
    Some(((days * 24 + h) * 60 + m) * 60 + sec)
}

/// `ps -o pid=,etime=`: each running PID's age in seconds.
#[must_use]
pub fn parse_etimes(text: &str) -> HashMap<u32, i64> {
    text.lines()
        .filter_map(|line| {
            let mut f = line.split_whitespace();
            Some((f.next()?.parse().ok()?, parse_etime(f.next()?)?))
        })
        .collect()
}

/// The cron section's answer: one job per label in `labels` that launchd has
/// loaded, in `labels`' order. `ages` is each running PID's age at `now`.
#[must_use]
pub fn jobs<S: std::hash::BuildHasher>(
    labels: &[String],
    rows: &[Row],
    ages: &HashMap<u32, i64, S>,
    now: i64,
) -> Value {
    let by_label: HashMap<&str, &Row> = rows.iter().map(|r| (r.label.as_str(), r)).collect();
    let jobs: Vec<Value> = labels
        .iter()
        .filter_map(|l| by_label.get(l.as_str()))
        .map(|r| {
            json!({
                "label": r.label,
                "pid": r.pid,
                "last_rc": r.status,
                "running_since": r.pid.and_then(|p| ages.get(&p)).map(|age| now - age),
                // launchd keeps no start time for a job that has finished, and
                // nothing on a Mac raises an alert for one.
                "last_start": Value::Null,
                "alert_since": Value::Null,
            })
        })
        .collect();
    json!({ "source": "launchd", "jobs": jobs })
}

#[cfg(test)]
#[path = "launchd_tests.rs"]
mod tests;
