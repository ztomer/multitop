//! Health score and alert checks per host.

use crate::config::Config;
use multitop_agent::render::Snapshot;

/// 0-100 health, where 100 is perfect.
///
/// `CPU>80` `MEM>85` `DSK>90` are the defaults from `config.example.toml` and
/// from `crates/agent/src/consts.rs` `CPU_HIGH_PCT` etc. A host with no
/// thresholds configured is a host the operator has not asked to watch, so it
/// scores 100 and never appears in `/ unhealthy`.
#[must_use]
pub fn health(snap: &Snapshot, cfg: &Config) -> u8 {
    let cpu_alert = cfg.alert_cpu.unwrap_or(80);
    let mem_alert = cfg.alert_mem.unwrap_or(85);
    let disk_alert = cfg.alert_disk.unwrap_or(90);

    let mut score: i16 = 100;

    // CPU busy percent across all cores.
    if snap.cpu_pct >= f64::from(cpu_alert) {
        score -= 30;
        if snap.cpu_pct >= 95.0 {
            score -= 10;
        }
    } else if snap.cpu_pct >= f64::from(cpu_alert).mul_add(0.75, 0.0) {
        score -= 10;
    }

    // Memory percent.
    let mem_pct = snap.mem.pct;
    if mem_pct >= f64::from(mem_alert) {
        score -= 30;
        if mem_pct >= 95.0 {
            score -= 10;
        }
    } else if mem_pct >= f64::from(mem_alert).mul_add(0.75, 0.0) {
        score -= 10;
    }

    // Disk percent.
    let disk_pct = snap.disk.pct;
    if disk_pct >= f64::from(disk_alert) {
        score -= 20;
        if disk_pct >= 95.0 {
            score -= 10;
        }
    }

    score.clamp(0, 100) as u8
}

/// Whether a snapshot breaches any configured threshold.
#[must_use]
pub fn is_breaching(snap: &Snapshot, cfg: &Config) -> bool {
    let cpu_alert = cfg.alert_cpu.unwrap_or(80);
    let mem_alert = cfg.alert_mem.unwrap_or(85);
    let disk_alert = cfg.alert_disk.unwrap_or(90);
    snap.cpu_pct >= f64::from(cpu_alert)
        || snap.mem.pct >= f64::from(mem_alert)
        || snap.disk.pct >= f64::from(disk_alert)
}

#[cfg(test)]
mod tests {
    use super::*;
    use multitop_agent::{proc::Usage, render::Snapshot};

    fn snap(cpu: f64, mem_pct: f64, disk_pct: f64) -> Snapshot {
        Snapshot {
            host: "test".into(),
            agent_version: "0.44.2".into(),
            cpu_pct: cpu,
            cpu_mhz: None,
            proc_names: vec![],
            cores: vec![],
            temp_unit: multitop_agent::render::TempUnit::C,
            mem: Usage::new(100, mem_pct as u64),
            disk: Usage::new(100, disk_pct as u64),
            rx_rate: 0.0,
            tx_rate: 0.0,
            procs: vec![],
        }
    }

    fn cfg() -> Config {
        Config {
            servers: vec![],
            theme: None,
            upgrade_history_lines: 5000,
            history_lines_raised_from: None,
            banner_style: Default::default(),
            plaintext_passwords: vec![],
            alert_cpu: Some(80),
            alert_mem: Some(85),
            alert_disk: Some(90),
        }
    }

    #[test]
    fn healthy_when_below_thresholds() {
        assert_eq!(health(&snap(10.0, 10.0, 10.0), &cfg()), 100);
    }

    #[test]
    fn breaching_when_any_over() {
        assert!(is_breaching(&snap(85.0, 10.0, 10.0), &cfg()));
        assert!(is_breaching(&snap(10.0, 90.0, 10.0), &cfg()));
        assert!(is_breaching(&snap(10.0, 10.0, 95.0), &cfg()));
        assert!(!is_breaching(&snap(10.0, 10.0, 10.0), &cfg()));
    }

    #[test]
    fn health_drops_on_breach() {
        assert!(health(&snap(85.0, 10.0, 10.0), &cfg()) < 100);
        assert!(health(&snap(95.0, 95.0, 95.0), &cfg()) < 50);
    }
}
