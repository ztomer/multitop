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
    /// Last selected panel, by `user@host:port`.
    pub selected_host: Option<String>,
    /// Last filter query.
    pub filter_query: Option<String>,
    /// Last sort mode (`cpu` or `mem`).
    pub sort: Option<String>,
    /// Per-host view (`monitor`/`docker`/`fetch`/`graphs`/`upgrade`), keyed by `user@host:port`.
    pub views: BTreeMap<String, String>,
    /// Saved filter queries, up to 3, via Ctrl-S.
    pub saved_filters: Vec<String>,
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

fn get_opt_string(val: &toml::Value, key: &str) -> Option<String> {
    val.as_table()
        .and_then(|t| t.get(key))
        .and_then(toml::Value::as_str)
        .map(ToString::to_string)
}

/// The state that was loaded, and anything the user has to be told about it.
///
/// A bare `AppState` could not distinguish "there is no history yet" from
/// "the history could not be read", and those are opposite facts. See
/// [`load_state`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StateLoad {
    pub state: AppState,
    /// Set when the file existed and could not be used. `None` on a clean load
    /// and on a first run.
    pub notice: Option<String>,
}

/// Read `state.toml`, and say so when it could not be read.
///
/// Returning `AppState::default()` for a corrupt or unreadable file made it
/// indistinguishable from a first run -- and the *next* `persist_state` then
/// wrote a fresh file straight over it, so the history was not merely ignored,
/// it was destroyed. [`write_atomic`] exists precisely so an interrupted write
/// cannot lose `upgrade_started_at`; the loader threw it away anyway on any
/// parse error. The writer was careful and the reader was not, which is where
/// this whole round keeps finding things.
///
/// An unreadable file is now moved aside rather than overwritten, so the next
/// write cannot destroy it and a human can still look at it.
#[must_use]
pub fn load_state(config_path: &Path) -> StateLoad {
    let path = state_file_path(config_path);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        // No file is the ordinary first run, and says nothing. Any other read
        // failure -- a permission change, an I/O error -- is not "no history
        // ever", and was reported as exactly that.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return StateLoad::default(),
        Err(e) => {
            return StateLoad {
                state: AppState::default(),
                notice: Some(format!(
                    "{} could not be read ({e}); upgrade history is unavailable this session.",
                    path.display()
                )),
            }
        }
    };
    let val = match toml::from_str::<toml::Value>(&text) {
        Ok(val) => val,
        Err(e) => {
            let kept = path.with_extension("toml.unreadable");
            let moved = std::fs::rename(&path, &kept).is_ok();
            return StateLoad {
                state: AppState::default(),
                notice: Some(if moved {
                    format!(
                        "{} could not be parsed ({e}); it has been kept as {} and a fresh one started.",
                        path.display(),
                        kept.display()
                    )
                } else {
                    format!(
                        "{} could not be parsed ({e}) and could not be moved aside; \
                         upgrade history is unavailable and will be overwritten.",
                        path.display()
                    )
                }),
            };
        }
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

    let selected_host = get_opt_string(&val, "selected_host");
    let filter_query = get_opt_string(&val, "filter_query");
    let sort = get_opt_string(&val, "sort");
    let mut views = BTreeMap::new();
    if let Some(table) = val
        .as_table()
        .and_then(|t| t.get("views"))
        .and_then(toml::Value::as_table)
    {
        for (k, v) in table {
            if let Some(s) = v.as_str() {
                views.insert(k.clone(), s.to_string());
            }
        }
    }

    let saved_filters = val
        .as_table()
        .and_then(|t| t.get("saved_filters"))
        .and_then(toml::Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(toml::Value::as_str)
                .map(ToString::to_string)
                .take(3)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    StateLoad {
        state: AppState {
            last_update,
            upgrade_started_at,
            hosts,
            selected_host,
            filter_query,
            sort,
            views,
            saved_filters,
        },
        notice: None,
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
    if let Some(s) = &state.selected_host {
        table.insert("selected_host".to_string(), toml::Value::String(s.clone()));
    }
    if let Some(s) = &state.filter_query {
        if !s.trim().is_empty() {
            table.insert("filter_query".to_string(), toml::Value::String(s.clone()));
        }
    }
    if let Some(s) = &state.sort {
        table.insert("sort".to_string(), toml::Value::String(s.clone()));
    }
    if !state.views.is_empty() {
        let mut views = toml::Table::new();
        for (k, v) in &state.views {
            views.insert(k.clone(), toml::Value::String(v.clone()));
        }
        table.insert("views".to_string(), toml::Value::Table(views));
    }
    if !state.saved_filters.is_empty() {
        let arr = state
            .saved_filters
            .iter()
            .take(3)
            .map(|s| toml::Value::String(s.clone()))
            .collect::<Vec<_>>();
        table.insert("saved_filters".to_string(), toml::Value::Array(arr));
    }

    let content = toml::to_string(&table).map_err(|e| e.to_string())?;
    write_atomic(&path, &content)
}

/// Write via a temporary file and a rename, so the destination is only ever the
/// old contents or the new ones.
///
/// Shared with `config`, which had the same hazard and none of the protection:
/// `config.toml` is written on a *keystroke* -- the theme and banner toggles --
/// and it is the file the user maintains by hand. A truncating write that is
/// interrupted there costs them the whole server list, which is strictly worse
/// than losing the state file this was originally built for.
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
pub(crate) fn write_atomic(path: &Path, content: &str) -> Result<(), String> {
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
            selected_host: None,
            filter_query: None,
            sort: None,
            views: BTreeMap::new(),
            saved_filters: Vec::new(),
        };

        save_state(&config_path, &state).unwrap();
        let loaded = load_state(&config_path);

        assert_eq!(loaded.state, state);
        assert_eq!(loaded.notice, None, "a clean load says nothing");

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
            selected_host: None,
            filter_query: None,
            sort: None,
            views: BTreeMap::new(),
            saved_filters: Vec::new(),
        };
        save_state(&config_path, &state).unwrap();
        let loaded = load_state(&config_path);

        assert_eq!(loaded.state, state);
        assert_eq!(loaded.notice, None, "a clean load says nothing");
        assert_eq!(loaded.state.hosts["admin@web-01:22"].outcome(), Outcome::Ok);
        assert_eq!(
            loaded.state.hosts["admin@web-01:22"].duration_secs(),
            Some(72)
        );
        assert_eq!(
            loaded.state.hosts["admin@db-02:22"].outcome(),
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
        assert_eq!(loaded.state.last_update, Some(1_722_000_000));
        assert_eq!(loaded.state.upgrade_started_at, Some(1_723_000_000));
        assert!(loaded.state.hosts.is_empty());

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
            selected_host: None,
            filter_query: None,
            sort: None,
            views: BTreeMap::new(),
            saved_filters: Vec::new(),
        };

        save_state(&config_path, &state).unwrap();
        let loaded = load_state(&config_path);

        assert_eq!(loaded.state, state);
        assert_eq!(loaded.notice, None, "a clean load says nothing");

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn layout_state_roundtrip() {
        let temp_dir = std::env::temp_dir().join("multitop_test_layout");
        let _ = std::fs::create_dir_all(&temp_dir);
        let config_path = temp_dir.join("config.toml");

        let mut views = BTreeMap::new();
        views.insert("ztomer@192.168.0.33:22".to_string(), "docker".to_string());
        views.insert("ztomer@192.168.0.90:22".to_string(), "fetch".to_string());

        let state = AppState {
            last_update: None,
            upgrade_started_at: None,
            hosts: BTreeMap::new(),
            selected_host: Some("ztomer@192.168.0.33:22".to_string()),
            filter_query: Some("beelink".to_string()),
            sort: Some("mem".to_string()),
            views,
            saved_filters: Vec::new(),
        };

        save_state(&config_path, &state).unwrap();
        let loaded = load_state(&config_path);

        assert_eq!(loaded.state, state);
        assert_eq!(
            loaded.state.selected_host.as_deref(),
            Some("ztomer@192.168.0.33:22")
        );
        assert_eq!(loaded.state.filter_query.as_deref(), Some("beelink"));
        assert_eq!(loaded.state.sort.as_deref(), Some("mem"));
        assert_eq!(loaded.state.views.len(), 2);
        assert_eq!(loaded.state.views["ztomer@192.168.0.33:22"], "docker");

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}

#[cfg(test)]
mod unreadable_state_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::{load_state, state_file_path, AppState};

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("multitop_state_{name}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("config.toml")
    }

    /// A corrupt state file must not read as a first run.
    ///
    /// It did, and that was worse than ignoring it: the next `persist_state`
    /// wrote a fresh file straight over it, so the history was destroyed rather
    /// than merely unread. `write_atomic` exists so an interrupted write cannot
    /// lose `upgrade_started_at`; the loader threw it away anyway.
    #[test]
    fn a_corrupt_state_file_is_kept_rather_than_overwritten() {
        let cfg = scratch("corrupt");
        let path = state_file_path(&cfg);
        std::fs::write(&path, "this is not = = toml [[[").unwrap();

        let loaded = load_state(&cfg);

        assert_eq!(
            loaded.state,
            AppState::default(),
            "nothing usable can be recovered from it"
        );
        let notice = loaded
            .notice
            .expect("a file that could not be parsed is not silence");
        assert!(
            notice.contains("could not be parsed"),
            "the notice must say what happened: {notice}"
        );

        let kept = path.with_extension("toml.unreadable");
        assert!(
            kept.exists(),
            "the unreadable file must be moved aside, or the next write destroys it"
        );
        assert!(
            !path.exists(),
            "and out of the way of that write: {}",
            path.display()
        );
        assert_eq!(
            std::fs::read_to_string(&kept).unwrap(),
            "this is not = = toml [[[",
            "kept verbatim, so a human can still look at it"
        );

        let _ = std::fs::remove_dir_all(cfg.parent().unwrap());
    }

    /// A first run is the one case that legitimately has no state, and it must
    /// stay silent -- a notice on every fresh install would train the user to
    /// ignore the line that matters.
    #[test]
    fn a_missing_state_file_says_nothing() {
        let cfg = scratch("missing");
        let loaded = load_state(&cfg);
        assert_eq!(loaded.state, AppState::default());
        assert_eq!(loaded.notice, None);
        let _ = std::fs::remove_dir_all(cfg.parent().unwrap());
    }
}
