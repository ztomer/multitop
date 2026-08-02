// macOS is Apple Silicon only. A compile error, not a runtime check, because
// Homebrew builds from source on the user's machine. The agent's x86-64 target
// is a different axis and is unaffected.
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
compile_error!("multitop requires Apple Silicon on macOS; Intel Macs are not supported");

pub mod ansi;
pub mod app;
pub mod config;
pub mod config_ui;
pub mod consts;
pub mod fetch_render;
pub mod fmt;
pub mod modals;
pub mod panel;
pub mod password_actions;
pub mod password_store;
pub mod passwords;
pub mod refit;
pub mod render_payload;
pub mod run;
pub mod sparkline;
pub mod ssh;
pub mod ssh_opts;
pub mod state;
pub mod stream;
pub mod tasks;
pub mod types;
pub mod ui;
pub mod upgrade_view;
pub mod vault;
