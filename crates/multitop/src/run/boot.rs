//! The `App` as it starts: what the config file and the state file each
//! contribute, the vault probe and the theme named on the command line.
//! Split from `event_loop.rs` for the line cap.

use crate::app::App;
use crate::config::Server;

/// What the config file contributes at startup: the history cap, the banner
/// style, the alert thresholds -- and the plaintext passwords it must not
/// keep. Passwords are loaded on-demand via `Panel::ensure_sudo_password()`
/// when the user initiates an upgrade, not here, so an app launch never
/// raises an OS keychain dialog.
fn apply_config(app: &mut App, config_path: &std::path::Path) {
    if let Ok(cfg) = crate::config::load(config_path) {
        app.upgrade_history_lines = cfg.upgrade_history_lines;
        app.banner_style = cfg.banner_style;
        // The ring was created at the default capacity; the config's value is
        // what the streaming log must honour.
        for p in &mut app.panels {
            p.last_upgrade.set_cap(cfg.upgrade_history_lines);
        }
        // A value below the floor was raised rather than obeyed, because
        // obeying it would have made the Upgrade pane silently swallow every
        // line -- including the warnings that name a lock file to remove. Said
        // out loud: the file still asks for something it will not get, and only
        // the user can put that right.
        if let Some(asked) = cfg.history_lines_raised_from {
            let note = format!(
                "config: upgrade_history_lines = {asked} would leave the Upgrade pane \
             with nothing to show; using {} instead.",
                cfg.upgrade_history_lines
            );
            for p in &mut app.panels {
                p.note(note.clone());
            }
        }
        // A sudo password in config.toml is plaintext on disk and was never
        // even read. Move any we find into the OS credential store and delete
        // them from the file, once, so the unsupported mechanism cannot linger.
        if !cfg.plaintext_passwords.is_empty() {
            crate::password_actions::port_plaintext_passwords(
                app,
                config_path,
                &cfg.plaintext_passwords,
            );
        }
        app.alert_cpu = cfg.alert_cpu;
        app.alert_mem = cfg.alert_mem;
        app.alert_disk = cfg.alert_disk;
        app.alert_targets = cfg.alerts;
    }
}

/// What the state file restores: the last update, the filter, the sort, the
/// selected host, each host's view, the saved filters -- and a notice when
/// the file could not be read, because that is not a first run.
fn restore_state(app: &mut App, config_path: &std::path::Path) {
    let loaded = crate::state::load_state(config_path);
    app.last_update = loaded.state.last_update;
    app.host_updates = loaded.state.hosts;
    app.upgrade_started_at = loaded.state.upgrade_started_at;
    if let Some(q) = loaded.state.filter_query {
        app.filter_query = q;
    }
    if let Some(s) = loaded.state.sort {
        match s.as_str() {
            "mem" | "memory" => app.sort = multitop_agent::SortBy::Mem,
            "cpu" => app.sort = multitop_agent::SortBy::Cpu,
            _ => {}
        }
    }
    if let Some(host) = loaded.state.selected_host {
        if let Some(idx) = app
            .panels
            .iter()
            .position(|p| crate::password_store::account(&p.server) == host)
        {
            app.selected_panel = idx;
        }
    }
    for (host, view_str) in loaded.state.views {
        if let Ok(mode) = view_str.parse::<crate::panel::Mode>() {
            if let Some(p) = app
                .panels
                .iter_mut()
                .find(|p| crate::password_store::account(&p.server) == host)
            {
                // Stored task-backed views restart as Monitor (see for_startup).
                p.mode = mode.for_startup();
            }
        }
    }
    app.saved_filters = loaded.state.saved_filters;
    // A state file that could not be read is not a first run, and saying
    // nothing made the two identical on screen while the next write destroyed
    // the evidence.
    if let Some(notice) = loaded.notice {
        for p in &mut app.panels {
            p.note(notice.clone());
        }
    }
}

/// The index of the theme named `name`, case-insensitively.
fn theme_index(name: &str) -> Option<usize> {
    multitop_agent::color::THEMES
        .iter()
        .position(|t| t.name.eq_ignore_ascii_case(name))
}

/// The `App` as it starts: the config's contribution, the state file's, the
/// vault probe and the theme named on the command line.
pub(super) fn boot_app(
    servers: &[Server],
    config_path: &std::path::Path,
    initial_theme: Option<&str>,
) -> App {
    boot_app_with(
        servers,
        config_path,
        initial_theme,
        crate::background::detect(),
    )
}

/// The boot path, with the background supplied.
///
/// The seam exists because the wiring is the part that breaks: for one commit
/// this detected a background, stored it, and then rendered with the view built
/// in `App::new` — before detection ran, against a default. Every module's own
/// test passed. A test that calls `rebuild_palette_view` itself cannot see that,
/// because it IS the rebuild, so the only way to test the connection is to drive
/// the function that makes it and let the background in from outside.
pub(super) fn boot_app_with(
    servers: &[Server],
    config_path: &std::path::Path,
    initial_theme: Option<&str>,
    background: crate::background::Background,
) -> App {
    let mut app = App::new(servers.to_vec());
    app.config_path = Some(config_path.to_path_buf());
    apply_config(&mut app, config_path);
    restore_state(&mut app, config_path);
    app.vault = crate::vault::create_vault(config_path).map(std::sync::Arc::new);
    if let Some(idx) = initial_theme.and_then(theme_index) {
        app.theme_idx = idx;
    }
    // Last, so it runs against a fully built App: the probe is a real terminal
    // read, and doing it after the theme is settled means the value is stored
    // once against the theme that will actually be used. The alternative --
    // querying per frame -- would put a terminal round trip in the render path
    // for an answer that cannot change while the process runs.
    // Detect, then REBUILD. The view was built in `App::new` against a default
    // background, so setting the detected one is not enough -- without this the
    // app detects the background correctly and then renders as though it had
    // not, and no test noticed, because nothing asserted the two were connected.
    app.background = background;
    app.rebuild_palette_view();
    app
}

#[cfg(test)]
mod tests {
    //! The boot path, driven end to end, because that is the only way to see the
    //! wiring.
    //!
    //! A test that calls `rebuild_palette_view` itself proves the function works
    //! and cannot prove anything calls it — and for one commit the app detected a
    //! background, stored it, and then rendered with a view built before the
    //! detection, while every one of those tests stayed green. The assertion
    //! here goes through `boot_app_with`, which is the function that has to make
    //! the connection, so removing the rebuild fails this test.

    use super::boot_app_with;
    use crate::background::Background;
    use crate::config::Server;
    use multitop_agent::adapt::MIN_TEXT_CONTRAST;
    use multitop_agent::surface::Rgb;

    /// A near-white terminal: the case the whole feature exists for.
    const LIGHT: Rgb = Rgb::new(252, 252, 250);

    fn config_in(dir: &std::path::Path) -> std::path::PathBuf {
        let path = dir.join("config.toml");
        std::fs::write(&path, "").expect("write an empty config");
        path
    }

    fn server(host: &str) -> Server {
        Server {
            host: host.to_string(),
            port: 0,
            user: "a".to_string(),
            upgrade_cmd: Some("true".to_string()),
            custom_command: None,
            mcp: None,
        }
    }

    #[test]
    fn the_boot_path_adapts_the_palette_to_the_background_it_detected() {
        let dir = tempfile::tempdir().expect("a temp dir");
        let app = boot_app_with(
            &[server("alpha")],
            &config_in(dir.path()),
            None,
            Background::Reported(LIGHT),
        );
        // Read the app's OWN view. A freshly built one would prove the module,
        // not the wiring.
        let drawn = app.current_theme();
        for (name, escape) in [
            ("primary", drawn.primary()),
            ("secondary", drawn.secondary()),
            ("muted", drawn.muted()),
            ("text", drawn.text()),
            ("meter_low", drawn.meter_low()),
            ("meter_mid", drawn.meter_mid()),
            ("meter_high", drawn.meter_high()),
        ] {
            let colour = Rgb::from_ansi(escape)
                .unwrap_or_else(|| panic!("{name} is not a colour: {escape:?}"));
            assert!(
                colour.contrast_ratio(LIGHT) >= MIN_TEXT_CONTRAST,
                "after boot on a light terminal, {name} is {:.2}:1: {escape:?}",
                colour.contrast_ratio(LIGHT)
            );
        }
    }

    #[test]
    fn boot_stores_the_background_it_was_given() {
        // The other half of the same connection, and the half that is easy to
        // believe without checking: a stored background nobody reads is the
        // silent version of the bug above.
        let dir = tempfile::tempdir().expect("a temp dir");
        let app = boot_app_with(
            &[server("alpha")],
            &config_in(dir.path()),
            None,
            Background::Reported(LIGHT),
        );
        assert_eq!(app.background, Background::Reported(LIGHT));
    }
}
