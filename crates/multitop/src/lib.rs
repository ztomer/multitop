// Apple Silicon only on macOS. Intel Macs are not supported.
//
// A compile error rather than a runtime check, so an unsupported build cannot
// be produced at all -- Homebrew builds this from source on the user's own
// machine, so without this an Intel Mac would happily compile a binary nobody
// tests or ships.
//
// This deliberately does NOT touch the x86-64 *agent*. The agent is a separate
// static musl binary uploaded to monitored Linux hosts, and dropping its
// x86-64 target would stop any Intel or AMD server being monitored. Same word,
// unrelated axis.
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
compile_error!(
    "multitop requires Apple Silicon on macOS; Intel Macs are not supported. \
     (Monitoring x86-64 Linux hosts is unaffected -- that is the agent's target, not this one.)"
);

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
