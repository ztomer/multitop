//! Render every screen the app can show, to files, at several terminal sizes.
//!
//! ```bash
//! cargo test --test render_views
//! # then read target/views/<cols>x<rows>/<screen>.txt
//! ```
//!
//! # Why this exists
//!
//! Reviewing anything visual meant launching the real app against real hosts
//! and taking a photograph of a terminal. So visual defects were found by the
//! user: a key hint truncated at 80 columns, a modal clipping its own footer,
//! a column that pushed the one beside it out of alignment. None of those need
//! a host, a network or a human -- they need the frame, written down.
//!
//! Two jobs, deliberately in one file:
//!
//! 1. **The harness.** Every screen at every size, as text, in a directory.
//!    That is the artifact a review reads.
//! 2. **The gate.** Every screen at every size must render without panicking,
//!    including sizes smaller than the content. `ui::draw` clips; a widget that
//!    computes a `Rect` wrongly panics inside ratatui, and until this existed
//!    only the Configuration panel's own sweep would have caught it.
//!
//! # Honest about what it is not
//!
//! The panel bodies here come from `multitop_agent::render` against a canned
//! `Snapshot` -- the same function a real agent's payload goes through, so the
//! stats layout is the real one -- but the *numbers* are invented and stable.
//! Nothing here proves anything about a live host.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;

use ratatui::backend::TestBackend;
use ratatui::Terminal;

use multitop::app::{App, AppMode, Mode, VaultState};
use multitop::config::Server;
use multitop::panel::UpgradeState;
use multitop::passwords::{PasswordEdit, PasswordManager, ServerDraft};

/// Terminal sizes worth looking at.
///
/// 80x24 is the size that has produced every truncation defect so far; 40x12 is
/// the "quarter of a small screen" a four-panel grid gives each host; the large
/// ones are where a layout that only works when cramped shows itself.
const SIZES: &[(u16, u16)] = &[(80, 24), (120, 35), (200, 50), (40, 12)];

fn server(host: &str, cmd: Option<&str>) -> Server {
    Server {
        host: host.to_string(),
        port: 22,
        user: "ztomer".to_string(),
        upgrade_cmd: cmd.map(str::to_string),
    }
}

/// A plausible, fixed snapshot. Real renderer, invented numbers.
fn snapshot(host: &str, cpu: f64) -> multitop_agent::render::Snapshot {
    multitop_agent::render::Snapshot {
        host: host.to_string(),
        agent_version: "0.24.0".to_string(),
        cpu_pct: cpu,
        cpu_mhz: Some(3600.0),
        // `u8` throughout so every widening is `f64::from`, which is lossless
        // and needs no cast lint silenced.
        cores: (0..8u8)
            .map(|i| {
                (
                    usize::from(i),
                    f64::from(i).mul_add(7.0, cpu) % 100.0,
                    Some(f64::from(i).mul_add(1.5, 41.0)),
                )
            })
            .collect(),
        temp_unit: multitop_agent::render::TempUnit::C,
        mem: multitop_agent::proc::Usage::new(32 * 1024 * 1024, 21 * 1024 * 1024),
        disk: multitop_agent::proc::Usage::new(500 * 1024 * 1024, 377 * 1024 * 1024),
        rx_rate: 184_320.0,
        tx_rate: 20_480.0,
        // PIDs written out rather than derived from the index, so there is no
        // index-to-u32 cast to justify.
        procs: [
            (1000u32, "postgres", 41.2, 2_411_724u64),
            (1137, "node", 18.7, 981_324),
            (1274, "dockerd", 9.1, 604_112),
            (1411, "nginx", 3.4, 88_204),
            (1548, "sshd", 0.9, 12_004),
            (1685, "systemd", 0.2, 9_880),
        ]
        .iter()
        .map(|(pid, name, cpu, mem)| multitop_agent::proc::Proc {
            pid: *pid,
            name: (*name).to_string(),
            cpu: *cpu,
            mem: *mem,
        })
        .collect(),
    }
}

/// Fill each panel's body the way a live agent frame would.
///
/// The size handed to the agent's renderer is the one `run`'s event loop hands
/// it -- `ui::agent_dims` for this terminal and this many panels -- not the
/// terminal's own size. Getting that wrong renders each host for a pane far
/// taller and wider than it gets, so the top of every panel is quietly cut off
/// and the frames misrepresent the product rather than reviewing it.
fn with_stats(app: &mut App, term: (u16, u16)) {
    let pal = &multitop_agent::color::ANSI;
    let (cols, rows) = multitop::ui::agent_dims(
        ratatui::layout::Size {
            width: term.0,
            height: term.1,
        },
        app.panels.len(),
    );
    let (cols, rows) = (usize::from(cols), usize::from(rows));
    for (i, p) in app.panels.iter_mut().enumerate() {
        // Each host gets a different, fixed load so the panels are telling
        // apart-able in a review; `u8` keeps the widening lossless.
        let spread = f64::from(u8::try_from(i).unwrap_or(0));
        let snap = snapshot(&p.server.host, spread.mul_add(31.0, 23.0));
        let lines = multitop_agent::render::render(
            &snap,
            cols,
            rows,
            multitop_agent::render::bar_len_for(cols),
            pal,
        );
        p.last_frame = Some(lines.clone());
        // Through the same method the app uses, so a pane's notices are drawn
        // here exactly as they are in the product. Assigning `view` directly is
        // how this file would go back to showing a frame the app never draws.
        p.show_frame(lines);
    }
}

fn hosts(n: usize) -> Vec<Server> {
    ["web-01", "db-02", "cache-03", "build-04"]
        .iter()
        .take(n)
        .map(|h| server(h, Some("sudo apt update && sudo apt upgrade -y")))
        .collect()
}

/// One screen: a name and the state that produces it.
struct Screen {
    name: &'static str,
    build: fn((u16, u16)) -> App,
}

fn base(n: usize, term: (u16, u16)) -> App {
    let mut app = App::new(hosts(n));
    app.config_path = Some(PathBuf::from("/tmp/multitop-render/config.toml"));
    with_stats(&mut app, term);
    app
}

fn settings(n: usize, term: (u16, u16)) -> App {
    let mut app = base(n, term);
    app.panels[0].sudo_password = Some("stored".to_string());
    app.panels[0].password_saved = true;
    app.password_manager = Some(PasswordManager::new(0, false));
    app
}

/// The vault handle only has to be `Some` for the screens that ask whether one
/// exists; nothing here unlocks or reads it.
fn with_vault(app: &mut App) {
    app.vault = Some(std::sync::Arc::new(multitop_vault::Vault::new(
        multitop_vault::VaultConfig {
            vault_path: PathBuf::from("/tmp/multitop-render/vault.bin"),
            argon2_params: None,
            use_os_keychain: false,
        },
    )));
}

#[rustfmt::skip]
const SCREENS: &[Screen] = &[
    Screen { name: "stats-1-host", build: |t| base(1, t) },
    Screen { name: "stats-2-hosts", build: |t| base(2, t) },
    Screen { name: "stats-4-hosts", build: |t| base(4, t) },
    // The Appearance opt-in, rendered. The banner is twice as wide per glyph
    // here, so this is where a centring bug would show as a rule that is longer
    // on one side than the other -- or as a banner that overruns its pane.
    Screen { name: "stats-wide-banner", build: |t| {
        let mut app = base(4, t);
        app.banner_style = multitop::layout::BannerStyle::Wide;
        app
    }},
    Screen { name: "settings-appearance", build: |_t| {
        let mut app = App::new(hosts(2));
        app.banner_style = multitop::layout::BannerStyle::Wide;
        multitop::passwords::open(&mut app, 0, false);
        app
    }},
    Screen { name: "stats-connecting", build: |_t| {
        let mut app = App::new(hosts(2));
        app.panels[1].view = vec!["connecting...".to_string()];
        app
    }},
    Screen { name: "filtering-typing", build: |t| {
        let mut app = base(4, t);
        app.mode = AppMode::Filtering;
        app.filter_query = "db".to_string();
        app
    }},
    Screen { name: "filtering-no-matches", build: |t| {
        let mut app = base(4, t);
        app.filter_query = "nothing-matches-this".to_string();
        app
    }},
    // A pane carrying a startup notice, at every size.
    //
    // Nothing rendered this before. The notices are written during startup --
    // the plaintext-password migration, a clamped `upgrade_history_lines`, an
    // unreadable `state.toml` -- and every one of them used to be erased by the
    // first agent frame, about a second later, because they were pushed into
    // `view`, which the frame rebuilds. A screen that draws one is how that
    // stays fixed.
    Screen { name: "monitor-with-notice", build: |t| {
        let mut app = base(2, t);
        for p in &mut app.panels {
            p.note(
                "config: upgrade_history_lines = 0 would leave the Upgrade pane \
                 with nothing to show; using 50 instead."
                    .to_string(),
            );
        }
        // The frame that used to wipe it.
        for p in &mut app.panels {
            p.show_last_frame();
        }
        app
    }},
    Screen { name: "upgrade-ready", build: |t| {
        let mut app = base(2, t);
        app.panels[0].password_saved = true;
        app.enter_upgrade_view();
        app
    }},
    Screen { name: "upgrade-no-command", build: |t| {
        let mut app = App::new(vec![server("web-01", None), server("db-02", Some("apt upgrade"))]);
        with_stats(&mut app, t);
        app.enter_upgrade_view();
        app
    }},
    // Driven through `confirm_upgrade`, the real production entry, rather than
    // by setting `upgrade_state` by hand.
    //
    // Hand-setting the flag left `host_updates` empty, so `host_update` returned
    // a default and this frame rendered "Last run  never" for a host that was
    // running -- a combination the app cannot actually produce. Production
    // writes `started_at: Some(now)` with no `finished_at` the moment a run
    // begins, which is *the same shape as an interrupted run*, and the header
    // read that shape as a verdict. Both this harness and the unit test beside
    // it modelled the same impossible state, which is how the header came to
    // say "Status running" and "Last run just now - interrupted" in
    // consecutive lines for ten passes without anyone seeing it.
    //
    // A harness that misrepresents the product is worse than none -- the same
    // lesson this file learned on its first run, when it fed the agent the
    // terminal size instead of the per-panel size.
    Screen { name: "upgrade-running", build: |t| {
        let mut app = base(2, t);
        app.enter_upgrade_view();
        let _ = app.confirm_upgrade();
        for p in &mut app.panels {
            p.upgrade_state = UpgradeState::STARTED;
            p.last_upgrade = vec![
                "Reading package lists...".to_string(),
                "Building dependency tree...".to_string(),
                "Get:1 http://archive.ubuntu.com noble InRelease [256 kB]".to_string(),
                "Unpacking libc6:amd64 (2.39-0ubuntu8.3) over (2.39-0ubuntu8.2)...".to_string(),
            ]
            .into();
        }
        app.mode = AppMode::Running;
        app
    }},
    // Scrolled back through a long upgrade log. The scroll badge belongs on
    // row 0 and, until row 0 had one owner, was built and destroyed on the same
    // frame -- so it had never appeared on screen.
    Screen { name: "upgrade-scrolled-back", build: |t| {
        let mut app = base(2, t);
        app.enter_upgrade_view();
        for p in &mut app.panels {
            p.upgrade_state = UpgradeState::STARTED;
            // The ring holds the log; the status header is composed over it at
            // draw time, and scrolling moves the ring's tail under the pinned
            // header. The scroll badge sits on row 0 and, until row 0 had one
            // owner, was built and destroyed on the same frame -- so it had
            // never appeared on screen.
            p.last_upgrade = (0..200)
                .map(|i| format!("Unpacking package-{i}..."))
                .collect::<Vec<String>>()
                .into();
            p.scroll_offset = 40;
        }
        app
    }},
    Screen { name: "upgrade-confirm-row", build: |t| {
        let mut app = base(2, t);
        app.enter_upgrade_view();
        app.set_show_upgrade_modal(true);
        app
    }},
    Screen { name: "docker", build: |t| {
        let mut app = base(2, t);
        for p in &mut app.panels {
            p.mode = Mode::Docker;
            p.view = vec![
                "CONTAINER      CPU%   MEM        STATUS".to_string(),
                "postgres-16    41.2%  2.3 GiB    Up 6 days".to_string(),
                "redis          1.1%   88 MiB     Up 6 days".to_string(),
                "nginx-proxy    0.4%   32 MiB     Up 2 hours (healthy)".to_string(),
            ];
        }
        app
    }},
    Screen { name: "settings-list", build: |t| settings(2, t) },
    Screen { name: "settings-row-editor", build: |t| {
        let mut app = settings(2, t);
        let panel = app.panels[0].server.clone();
        app.password_manager.as_mut().unwrap().draft =
            Some(ServerDraft::new(Some(0), Some(&panel), Some("stored-secret")));
        app
    }},
    Screen { name: "settings-delete-confirm", build: |t| {
        let mut app = settings(2, t);
        let m = app.password_manager.as_mut().unwrap();
        m.pending_delete = Some(0);
        m.notice = Some("Remove web-01 from the configuration? [y] confirm  [Esc] cancel".to_string());
        app
    }},
    Screen { name: "settings-rotate-prompt", build: |t| {
        let mut app = settings(2, t);
        with_vault(&mut app);
        let m = app.password_manager.as_mut().unwrap();
        m.edit = Some(PasswordEdit::RotateCurrent);
        m.input = "hunter2".to_string();
        m.notice = Some("Enter the CURRENT master password:".to_string());
        app
    }},
    Screen { name: "vault-create-prompt", build: |t| {
        let mut app = settings(2, t);
        app.vault_state = VaultState::Creating { error: None, in_flight: false };
        *app.vault_password_input_mut() = "chosen-master".to_string();
        app
    }},
    Screen { name: "vault-creating", build: |t| {
        let mut app = settings(2, t);
        app.vault_state = VaultState::Creating { error: None, in_flight: true };
        app
    }},
    Screen { name: "vault-create-error", build: |t| {
        let mut app = settings(2, t);
        app.vault_state = VaultState::Creating {
            error: Some("Master password cannot be empty".to_string()),
            in_flight: false,
        };
        app
    }},
    Screen { name: "vault-unlock-prompt", build: |t| {
        let mut app = base(2, t);
        with_vault(&mut app);
        app.vault_state = VaultState::PasswordPrompt { error: None };
        *app.vault_password_input_mut() = "master".to_string();
        app
    }},
    Screen { name: "vault-unlock-error", build: |t| {
        let mut app = base(2, t);
        with_vault(&mut app);
        app.vault_state = VaultState::PasswordPrompt {
            error: Some("Rate limited: try again in 30s".to_string()),
        };
        app
    }},
    Screen { name: "vault-biometric-wait", build: |t| {
        let mut app = base(2, t);
        with_vault(&mut app);
        app.vault_state = VaultState::Unlocking { awaiting_biometric: true };
        app
    }},
    Screen { name: "vault-verifying", build: |t| {
        let mut app = base(2, t);
        with_vault(&mut app);
        app.vault_state = VaultState::Unlocking { awaiting_biometric: false };
        app
    }},
];

/// The buffer as plain text, one string per row, escapes resolved by ratatui.
fn frame(term: &Terminal<TestBackend>) -> String {
    let buf = term.backend().buffer();
    let width = buf.area.width as usize;
    buf.content()
        .iter()
        .map(ratatui::buffer::Cell::symbol)
        .collect::<Vec<_>>()
        .chunks(width)
        .map(<[&str]>::concat)
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn every_screen_renders_at_every_size() {
    // Nothing here may reach the real credential store: `enter_upgrade_view`
    // and `PasswordManager` both go through `password_store` several calls
    // down.
    let _guard = multitop::password_store::lock_for_test();
    multitop::password_store::enable_mock_store();
    multitop::password_store::clear_mock_store();

    // Anchored to the manifest, not the working directory. `canonicalize` was
    // tried first and cannot be: it fails on a path that does not exist yet, so
    // the fallback fired and the frames landed in `crates/multitop/target`,
    // where nobody would look for them.
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/views");
    // Cleared, not merely overwritten. A screen that is deleted from `SCREENS`
    // used to leave its last frame lying in `target/views` forever, so the
    // directory a reviewer reads as "what the product looks like" kept showing
    // screens the product no longer has -- `upgrade-confirm-modal.txt` outlived
    // the modal by a whole review round. A harness that misrepresents the
    // product is worse than none, which this harness has already been told once.
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();

    for &(cols, rows) in SIZES {
        let dir = root.join(format!("{cols}x{rows}"));
        std::fs::create_dir_all(&dir).unwrap();
        for screen in SCREENS {
            let mut app = (screen.build)((cols, rows));
            let mut term = Terminal::new(TestBackend::new(cols, rows)).unwrap();
            // The assertion is that this returns. A widget given a `Rect`
            // outside the frame panics inside ratatui rather than clipping.
            term.draw(|f| multitop::ui::draw(f, &mut app))
                .unwrap_or_else(|e| panic!("{} at {cols}x{rows}: {e}", screen.name));
            std::fs::write(dir.join(format!("{}.txt", screen.name)), frame(&term)).unwrap();
        }
    }

    println!(
        "wrote {} screens x {} sizes to {}",
        SCREENS.len(),
        SIZES.len(),
        root.display()
    );
}
