// Configuration types and constants.

use std::path::PathBuf;

pub const EXAMPLE_CONFIG: &str = include_str!("../../../../config.example.toml");
pub const DEFAULT_PORT: u16 = 22;
pub const DEFAULT_UPGRADE_HISTORY_LINES: usize = 5000;
pub const MIN_UPGRADE_HISTORY_LINES: usize = 50;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigError(pub String);

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ConfigError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Server {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub upgrade_cmd: Option<String>,
}

impl Server {
    /// SSH destination, `user@host` when a user is configured.
    #[must_use]
    pub fn target(&self) -> std::borrow::Cow<'_, str> {
        if self.user.is_empty() {
            std::borrow::Cow::Borrowed(&self.host)
        } else {
            std::borrow::Cow::Owned(format!("{}@{}", self.user, self.host))
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Config {
    pub servers: Vec<Server>,
    pub theme: Option<String>,
    pub upgrade_history_lines: usize,
    pub history_lines_raised_from: Option<usize>,
    pub banner_style: crate::layout::BannerStyle,
    pub plaintext_passwords: Vec<(Server, String)>,
}

pub fn default_config_path() -> PathBuf {
    std::env::var("HOME")
        .map_or_else(|_| PathBuf::from("."), PathBuf::from)
        .join(".config/multitop/config.toml")
}
