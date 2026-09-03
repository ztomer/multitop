//! Health score and alert checks per host.

use crate::config::Config;
use multitop_agent::render::Snapshot;

/// 0-100 health, where 100 is perfect.
///
/// `CPU>80` `MEM>85` `DSK>90` are the defaults from `config.example.toml` and
/// from `crates/agent/src/consts.rs` `CPU_HIGH_PCT` etc. A host with no
/// thresholds configured is a host the operator has not asked to watch, so it
/// scores 100 and never appears in `/ unhealthy`.
pub const MAX_HEALTH: u8 = 100;
pub const HEALTH_RED_BELOW: u8 = 50;
pub const HEALTH_YELLOW_BELOW: u8 = 80;
const CRITICAL_PCT: f64 = 95.0;
const WARNING_FACTOR: f64 = 0.75;
const CPU_ALERT_PENALTY: i16 = 30;
const MEM_ALERT_PENALTY: i16 = 30;
const DISK_ALERT_PENALTY: i16 = 20;
const CRITICAL_EXTRA_PENALTY: i16 = 10;
const NEAR_ALERT_PENALTY: i16 = 10;
const VAULT_PENALTY: i16 = 15;
const NET_PENALTY: i16 = 10;
/// 50 MiB/s combined is a saturated link on many hosts.
const NET_HIGH_BYTES_PER_SEC: f64 = 50.0 * 1024.0 * 1024.0;

#[must_use]
pub fn health(snap: &Snapshot, cfg: &Config) -> u8 {
    health_with_vault(snap, cfg, false)
}

/// Same as `health` but penalises a locked vault and a saturated link.
///
/// `vault_locked` is the client-side signal "passwords exist but `vault` is
/// `Locked`": the host is operable but upgrades will prompt. `NET` is `rx+tx`.
/// Failed systemd units and pending upgrades are not yet in `Snapshot`; they
/// will subtract here once the agent ships them, same shape as `VAULT_PENALTY`.
#[must_use]
pub fn health_with_vault(snap: &Snapshot, cfg: &Config, vault_locked: bool) -> u8 {
    let cpu_alert = cfg.alert_cpu.unwrap_or(80);
    let mem_alert = cfg.alert_mem.unwrap_or(85);
    let disk_alert = cfg.alert_disk.unwrap_or(90);

    let mut score: i16 = i16::from(MAX_HEALTH);

    // CPU busy percent across all cores.
    if snap.cpu_pct >= f64::from(cpu_alert) {
        score -= CPU_ALERT_PENALTY;
        if snap.cpu_pct >= CRITICAL_PCT {
            score -= CRITICAL_EXTRA_PENALTY;
        }
    } else if snap.cpu_pct >= f64::from(cpu_alert).mul_add(WARNING_FACTOR, 0.0) {
        score -= NEAR_ALERT_PENALTY;
    }

    // Memory percent.
    let mem_pct = snap.mem.pct;
    if mem_pct >= f64::from(mem_alert) {
        score -= MEM_ALERT_PENALTY;
        if mem_pct >= CRITICAL_PCT {
            score -= CRITICAL_EXTRA_PENALTY;
        }
    } else if mem_pct >= f64::from(mem_alert).mul_add(WARNING_FACTOR, 0.0) {
        score -= NEAR_ALERT_PENALTY;
    }

    // Disk percent.
    let disk_pct = snap.disk.pct;
    if disk_pct >= f64::from(disk_alert) {
        score -= DISK_ALERT_PENALTY;
        if disk_pct >= CRITICAL_PCT {
            score -= CRITICAL_EXTRA_PENALTY;
        }
    }

    if vault_locked {
        score -= VAULT_PENALTY;
    }
    if snap.rx_rate + snap.tx_rate >= NET_HIGH_BYTES_PER_SEC {
        score -= NET_PENALTY;
    }

    u8::try_from(score.clamp(0, i16::from(MAX_HEALTH))).unwrap_or(0)
}

/// Whether a snapshot breaches any configured threshold.
#[must_use]
pub fn is_breaching(snap: &Snapshot, cfg: &Config) -> bool {
    is_breaching_with_vault(snap, cfg, false)
}

#[must_use]
pub fn is_breaching_with_vault(snap: &Snapshot, cfg: &Config, vault_locked: bool) -> bool {
    let cpu_alert = cfg.alert_cpu.unwrap_or(80);
    let mem_alert = cfg.alert_mem.unwrap_or(85);
    let disk_alert = cfg.alert_disk.unwrap_or(90);
    snap.cpu_pct >= f64::from(cpu_alert)
        || snap.mem.pct >= f64::from(mem_alert)
        || snap.disk.pct >= f64::from(disk_alert)
        || vault_locked
}

#[cfg(test)]
mod tests {
    use super::*;
    use multitop_agent::{proc::Usage, render::Snapshot};

    fn snap(cpu: f64, mem_pct: u64, disk_pct: u64) -> Snapshot {
        Snapshot {
            host: "test".into(),
            agent_version: "0.44.2".into(),
            cpu_pct: cpu,
            cpu_mhz: None,
            proc_names: vec![],
            cores: vec![],
            temp_unit: multitop_agent::render::TempUnit::C,
            mem: Usage::new(100, mem_pct),
            disk: Usage::new(100, disk_pct),
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
            banner_style: crate::layout::BannerStyle::default(),
            plaintext_passwords: vec![],
            alert_cpu: Some(80),
            alert_mem: Some(85),
            alert_disk: Some(90),
            alerts: vec![],
        }
    }

    #[test]
    fn healthy_when_below_thresholds() {
        assert_eq!(health(&snap(10.0, 10, 10), &cfg()), 100);
    }

    #[test]
    fn breaching_when_any_over() {
        assert!(is_breaching(&snap(85.0, 10, 10), &cfg()));
        assert!(is_breaching(&snap(10.0, 90, 10), &cfg()));
        assert!(is_breaching(&snap(10.0, 10, 95), &cfg()));
        assert!(!is_breaching(&snap(10.0, 10, 10), &cfg()));
    }

    #[test]
    fn health_drops_on_breach() {
        assert!(health(&snap(85.0, 10, 10), &cfg()) < MAX_HEALTH);
        assert!(health(&snap(95.0, 95, 95), &cfg()) < HEALTH_RED_BELOW);
    }

    #[test]
    fn vault_locked_costs_health_and_counts_as_unhealthy() {
        let s = snap(10.0, 10, 10);
        assert_eq!(health(&s, &cfg()), 100);
        assert_eq!(health_with_vault(&s, &cfg(), true), 85);
        assert!(!is_breaching(&s, &cfg()));
        assert!(is_breaching_with_vault(&s, &cfg(), true));
    }

    #[test]
    fn net_traffic_penalises_health() {
        let mut s = snap(10.0, 10, 10);
        s.rx_rate = 30.0 * 1024.0 * 1024.0;
        s.tx_rate = 30.0 * 1024.0 * 1024.0;
        assert!(health_with_vault(&s, &cfg(), false) < 100);
        assert!(!is_breaching_with_vault(&s, &cfg(), false));
    }
}
