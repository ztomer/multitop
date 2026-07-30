//! Persistent runtime state stored in `state.toml` next to the configuration file.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AppState {
    pub last_update: Option<u64>,
    pub upgrade_started_at: Option<u64>,
}

pub fn state_file_path(config_path: &Path) -> PathBuf {
    config_path.with_file_name("state.toml")
}

fn get_opt_u64(val: &toml::Value, key: &str) -> Option<u64> {
    val.as_table()
        .and_then(|t| t.get(key))
        .and_then(|v| v.as_integer())
        .map(|n| n as u64)
}

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
    AppState {
        last_update,
        upgrade_started_at,
    }
}

fn insert_opt_u64(table: &mut toml::Table, key: &str, val: Option<u64>) {
    if let Some(v) = val {
        table.insert(key.to_string(), toml::Value::Integer(v as i64));
    }
}

pub fn save_state(config_path: &Path, state: &AppState) -> Result<(), String> {
    let path = state_file_path(config_path);
    let mut table = toml::Table::new();
    insert_opt_u64(&mut table, "last_update", state.last_update);
    insert_opt_u64(&mut table, "upgrade_started_at", state.upgrade_started_at);
    let content = toml::to_string(&table).map_err(|e| e.to_string())?;
    std::fs::write(&path, content).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_save_and_load_roundtrip() {
        let temp_dir = std::env::temp_dir().join("multitop_test_state");
        let _ = std::fs::create_dir_all(&temp_dir);
        let config_path = temp_dir.join("config.toml");

        let state = AppState {
            last_update: Some(1722000000),
            upgrade_started_at: None,
        };

        save_state(&config_path, &state).unwrap();
        let loaded = load_state(&config_path);

        assert_eq!(loaded, state);

        let _ = std::fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn upgrade_started_at_roundtrip() {
        let temp_dir = std::env::temp_dir().join("multitop_test_started");
        let _ = std::fs::create_dir_all(&temp_dir);
        let config_path = temp_dir.join("config.toml");

        let state = AppState {
            last_update: None,
            upgrade_started_at: Some(1723000000),
        };

        save_state(&config_path, &state).unwrap();
        let loaded = load_state(&config_path);

        assert_eq!(loaded, state);

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
