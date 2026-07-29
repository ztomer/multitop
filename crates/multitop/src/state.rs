//! Persistent runtime state stored in `state.toml` next to the configuration file.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AppState {
    pub last_update: Option<String>,
}

pub fn state_file_path(config_path: &Path) -> PathBuf {
    config_path.with_file_name("state.toml")
}

pub fn load_state(config_path: &Path) -> AppState {
    let path = state_file_path(config_path);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return AppState::default();
    };
    let Ok(val) = toml::from_str::<toml::Value>(&text) else {
        return AppState::default();
    };
    let last_update = val
        .as_table()
        .and_then(|t| t.get("last_update"))
        .and_then(|v| v.as_str())
        .map(String::from);
    AppState { last_update }
}

pub fn save_state(config_path: &Path, state: &AppState) -> Result<(), String> {
    let path = state_file_path(config_path);
    let mut table = toml::Table::new();
    if let Some(ref lu) = state.last_update {
        table.insert("last_update".to_string(), toml::Value::String(lu.clone()));
    }
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
            last_update: Some("2026-07-29 17:46:00".to_string()),
        };

        save_state(&config_path, &state).unwrap();
        let loaded = load_state(&config_path);

        assert_eq!(loaded, state);

        let _ = std::fs::remove_dir_all(&temp_dir);
    }
}
