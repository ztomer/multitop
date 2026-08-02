//! Persistent runtime state stored in `state.toml` next to the configuration file.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// What happened the last time an upgrade ran on one host.
///
/// `finished_at` being `None` while `started_at` is set means the run never
/// reported back — the app was killed, or the connection dropped mid-upgrade.
/// That is worth telling the user about before they run it again.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HostUpdate {
    pub started_at: Option<u64>,
    pub finished_at: Option<u64>,
    pub success: bool,
}

/// How a host's last upgrade ended, for display.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Never,
    Ok,
    Failed,
    Interrupted,
}

impl HostUpdate {
    #[must_use]
    pub const fn outcome(&self) -> Outcome {
        match (self.started_at, self.finished_at) {
            (None, None) => Outcome::Never,
            (Some(_), None) => Outcome::Interrupted,
            _ if self.success => Outcome::Ok,
            _ => Outcome::Failed,
        }
    }

    /// Wall-clock duration of the last completed run.
    #[must_use]
    pub const fn duration_secs(&self) -> Option<u64> {
        match (self.started_at, self.finished_at) {
            (Some(s), Some(f)) if f >= s => Some(f - s),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AppState {
    pub last_update: Option<u64>,
    pub upgrade_started_at: Option<u64>,
    /// Per-host history, keyed by `user@host:port` (see
    /// `password_store::account`). Keyed by stable identity rather than panel
    /// index so records survive reordering or adding servers.
    pub hosts: BTreeMap<String, HostUpdate>,
}

#[must_use]
pub fn state_file_path(config_path: &Path) -> PathBuf {
    config_path.with_file_name("state.toml")
}

fn get_opt_u64(val: &toml::Value, key: &str) -> Option<u64> {
    val.as_table()
        .and_then(|t| t.get(key))
        .and_then(toml::Value::as_integer)
        .and_then(|n| u64::try_from(n).ok())
}

#[must_use]
pub fn load_state(config_path: &Path) -> AppState {
    let path = state_file_path(config_path);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return AppState::default();
    };
    let Ok(val) = toml::from_str::<toml::Value>(&text) else {
        return AppState::default();
    };
    let last_update = get_opt_u64(&val, "last_update");
    let upgrade_started_at = get_opt_u64(&val, "upgrade_started_at");

    // A state.toml written before per-host records existed simply has no
    // [hosts] table; it must still load rather than resetting the file.
    let mut hosts = BTreeMap::new();
    if let Some(table) = val
        .as_table()
        .and_then(|t| t.get("hosts"))
        .and_then(toml::Value::as_table)
    {
        for (key, entry) in table {
            hosts.insert(
                key.clone(),
                HostUpdate {
                    started_at: get_opt_u64(entry, "started_at"),
                    finished_at: get_opt_u64(entry, "finished_at"),
                    success: entry
                        .as_table()
                        .and_then(|t| t.get("success"))
                        .and_then(toml::Value::as_bool)
                        .unwrap_or(false),
                },
            );
        }
    }

    AppState {
        last_update,
        upgrade_started_at,
        hosts,
    }
}

#[allow(clippy::expect_used)]
fn insert_opt_u64(table: &mut toml::Table, key: &str, val: Option<u64>) {
    if let Some(v) = val {
        table.insert(
            key.to_string(),
            toml::Value::Integer(i64::try_from(v).expect("u64 fits in i64")),
        );
    }
}

/// Save the application state to a TOML file.
///
/// # Errors
///
/// Returns an error if the file cannot be written.
pub fn save_state(config_path: &Path, state: &AppState) -> Result<(), String> {
    let path = state_file_path(config_path);
    let mut table = toml::Table::new();
    insert_opt_u64(&mut table, "last_update", state.last_update);
    insert_opt_u64(&mut table, "upgrade_started_at", state.upgrade_started_at);

    if !state.hosts.is_empty() {
        let mut hosts = toml::Table::new();
        for (key, rec) in &state.hosts {
            let mut entry = toml::Table::new();
            insert_opt_u64(&mut entry, "started_at", rec.started_at);
            insert_opt_u64(&mut entry, "finished_at", rec.finished_at);
            entry.insert("success".to_string(), toml::Value::Boolean(rec.success));
            hosts.insert(key.clone(), toml::Value::Table(entry));
        }
        table.insert("hosts".to_string(), toml::Value::Table(hosts));
    }

    let content = toml::to_string(&table).map_err(|e| e.to_string())?;
    write_atomic(&path, &content)
}

/// Write via a temporary file and a rename, so the destination is only ever the
/// old contents or the new ones.
///
/// `fs::write` truncates before it writes, so an interruption does not merely
/// fail to record the new value -- it destroys the previous one. That matters
/// here more than most places: `upgrade_started_at` exists so an upgrade cut
/// short by a power loss can be reported afterwards, and it is written at the
/// moment the upgrade starts. Losing power during that write left an empty file,
/// which loads as "no state at all", so the record that power-loss detection
/// depends on was itself the thing power loss erased.
///
/// The temporary name carries the pid so two instances cannot write the same
/// scratch file, and it is removed on every failure path -- a leftover would
/// otherwise accumulate beside the config forever.
fn write_atomic(path: &Path, content: &str) -> Result<(), String> {
    use std::io::Write as _;

    let tmp = path.with_extension(format!("toml.{}.tmp", std::process::id()));
    let result = (|| -> std::io::Result<()> {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(content.as_bytes())?;
        // Flush the contents before the rename publishes them; a rename of a
        // file whose data has not reached the disk can still surface as empty
        // after a crash.
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp, path)?;
        // Persist the rename itself, for the same reason.
        if let Some(parent) = path.parent() {
            if let Ok(dir) = std::fs::File::open(parent) {
                let _ = dir.sync_all();
            }
        }
        Ok(())
    })();

    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result.map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;

    #[test]
    fn state_save_and_load_roundtrip() {
        let temp_dir = std::env::temp_dir().join("multitop_test_state");
        let _ = std::fs::create_dir_all(&temp_dir);
        let config_path = temp_dir.join("config.toml");

        let state = AppState {
            last_update: Some(1_722_000_000),
            upgrade_started_at: None,
            hosts: BTreeMap::new(),
        };

        save_state(&config_path, &state).unwrap();
        let loaded = load_state(&config_path);

        assert_eq!(loaded, state);

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn per_host_records_roundtrip() {
        let temp_dir = std::env::temp_dir().join("multitop_test_state_hosts");
        let _ = std::fs::create_dir_all(&temp_dir);
        let config_path = temp_dir.join("config.toml");

        let mut hosts = BTreeMap::new();
        hosts.insert(
            "admin@web-01:22".to_string(),
            HostUpdate {
                started_at: Some(1_722_000_000),
                finished_at: Some(1_722_000_072),
                success: true,
            },
        );
        // An interrupted run: started, never finished.
        hosts.insert(
            "admin@db-02:22".to_string(),
            HostUpdate {
                started_at: Some(1_722_000_000),
                finished_at: None,
                success: false,
            },
        );

        let state = AppState {
            last_update: Some(1_722_000_072),
            upgrade_started_at: None,
            hosts,
        };
        save_state(&config_path, &state).unwrap();
        let loaded = load_state(&config_path);

        assert_eq!(loaded, state);
        assert_eq!(loaded.hosts["admin@web-01:22"].outcome(), Outcome::Ok);
        assert_eq!(loaded.hosts["admin@web-01:22"].duration_secs(), Some(72));
        assert_eq!(
            loaded.hosts["admin@db-02:22"].outcome(),
            Outcome::Interrupted
        );

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    /// A state.toml written before per-host records existed must still load.
    #[test]
    fn legacy_state_file_without_hosts_still_loads() {
        let temp_dir = std::env::temp_dir().join("multitop_test_state_legacy");
        let _ = std::fs::create_dir_all(&temp_dir);
        let config_path = temp_dir.join("config.toml");
        std::fs::write(
            state_file_path(&config_path),
            "last_update = 1722000000\nupgrade_started_at = 1723000000\n",
        )
        .unwrap();

        let loaded = load_state(&config_path);
        assert_eq!(loaded.last_update, Some(1_722_000_000));
        assert_eq!(loaded.upgrade_started_at, Some(1_723_000_000));
        assert!(loaded.hosts.is_empty());

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn outcome_classifies_every_combination() {
        assert_eq!(HostUpdate::default().outcome(), Outcome::Never);
        assert_eq!(
            HostUpdate {
                started_at: Some(1),
                finished_at: None,
                success: false
            }
            .outcome(),
            Outcome::Interrupted
        );
        assert_eq!(
            HostUpdate {
                started_at: Some(1),
                finished_at: Some(2),
                success: true
            }
            .outcome(),
            Outcome::Ok
        );
        assert_eq!(
            HostUpdate {
                started_at: Some(1),
                finished_at: Some(2),
                success: false
            }
            .outcome(),
            Outcome::Failed
        );
    }

    #[test]
    fn upgrade_started_at_roundtrip() {
        let temp_dir = std::env::temp_dir().join("multitop_test_started");
        let _ = std::fs::create_dir_all(&temp_dir);
        let config_path = temp_dir.join("config.toml");

        let state = AppState {
            last_update: None,
            upgrade_started_at: Some(1_723_000_000),
            hosts: BTreeMap::new(),
        };

        save_state(&config_path, &state).unwrap();
        let loaded = load_state(&config_path);

        assert_eq!(loaded, state);

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
