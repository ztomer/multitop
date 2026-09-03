//! Configuration: load, save, validate, parse.

mod load;
mod save;
mod ssh;
mod types;

pub use load::{load, parse, validate_host, validate_user};
pub use save::{save_banner_style, save_servers, save_theme, strip_plaintext_passwords};
pub use ssh::{merge_ssh_hosts, parse_ssh_config, ssh_config_path};
pub use types::{
    default_config_path, AlertTarget, Config, ConfigError, Server, DEFAULT_PORT,
    DEFAULT_UPGRADE_HISTORY_LINES, EXAMPLE_CONFIG, MIN_UPGRADE_HISTORY_LINES,
};
