//! Every glyph the app draws, measured against the background it is drawn ON.
//!
//! The command bar shipped near-white on dark terminals in every theme from
//! v0.50.0 to v0.53.0: its background was built from the adapted TEXT colour,
//! and its text was named terminal colours (`Color::White`, `DarkGray`), so it
//! was white on near-white -- 1.02:1 to 1.73:1. Nothing saw it, because every
//! instrument read only foregrounds: the palette tests measured each role
//! against the terminal background, `light_terminal_check.py` read only
//! `38;2` escapes, and the bar's background was never in any measurement.
//!
//! This reads the CELL: the real `ui::draw`, into a `TestBackend`, for every
//! theme on a dark terminal, a light one and one that did not answer, in each
//! state that changes what the chrome draws. Each non-blank cell's foreground
//! is measured against that cell's own background, with `Reset` resolved to the
//! terminal's. A named or indexed colour fails outright: the terminal decides
//! what it looks like, so no palette can promise it reads.
//!
//! The floors are the palette's own: bold glyphs (keys, titles, the active
//! mode) are text and need `MIN_TEXT_CONTRAST`; the rest may be de-emphasised
//! and need `MIN_MUTED_CONTRAST`.

#![cfg(test)]

#[path = "common/mod.rs"]
mod common;

use std::collections::BTreeSet;
use std::sync::Arc;

use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use multitop::app::{App, Msg};
use multitop::background::Background;
use multitop::config::Server;
use multitop::panel::{Mode, UpgradeState};
use multitop::password_store;
use multitop::run::{handle_key, Tasks};
use multitop_agent::adapt::{MIN_MUTED_CONTRAST, MIN_TEXT_CONTRAST};
use multitop_agent::color::THEMES;
use multitop_agent::proc::{Proc, Usage};
use multitop_agent::proto::Payload;
use multitop_agent::render::{Snapshot, TempUnit};
use multitop_agent::surface::Rgb;
use ratatui::backend::TestBackend;
use ratatui::style::{Color, Modifier};
use ratatui::Terminal;
use tokio::sync::{mpsc, watch};

const SIZE: (u16, u16) = (120, 32);

/// The terminals: what it reports, the background it paints, and the default
/// foreground it draws `Reset` text in.
fn terminals() -> [(&'static str, Background, Option<Rgb>, Rgb); 3] {
    [
        (
            "dark",
            Background::Reported(Rgb::new(30, 30, 30)),
            Some(Rgb::new(30, 30, 30)),
            Rgb::new(229, 229, 229),
        ),
        (
            "light",
            Background::Reported(Rgb::new(255, 255, 255)),
            Some(Rgb::new(255, 255, 255)),
            Rgb::new(0, 0, 0),
        ),
        // Nothing answered: the app assumes the theme's own background, and so
        // does this measurement -- it is the only background the theme states.
        (
            "unknown",
            Background::Unknown,
            None,
            Rgb::new(229, 229, 229),
        ),
    ]
}

/// A state of the UI, reached the way a user reaches it: by keys -- with, for
/// the views that have one, the agent's frame already arrived. Without a frame
/// the panel shows only its placeholder, and the stats body, its refitted
/// header and rules, and the docker and fetch views go unmeasured: the refit
/// rule was terminal grey (`ESC[90m`) for exactly that reason.
struct State {
    name: &'static str,
    keys: &'static [KeyCode],
    upgrade_running: bool,
    view: Option<fn() -> (Mode, Payload)>,
}

const STATES: &[State] = &[
    State {
        name: "stats",
        keys: &[],
        upgrade_running: false,
        view: None,
    },
    State {
        name: "filter typing",
        keys: &[KeyCode::Char('/'), KeyCode::Char('z')],
        upgrade_running: false,
        view: None,
    },
    State {
        name: "no host matches",
        keys: &[KeyCode::Char('/'), KeyCode::Char('z'), KeyCode::Enter],
        upgrade_running: false,
        view: None,
    },
    State {
        name: "help",
        keys: &[KeyCode::Char('?')],
        upgrade_running: false,
        view: None,
    },
    State {
        name: "palette",
        keys: &[KeyCode::Char(':')],
        upgrade_running: false,
        view: None,
    },
    State {
        name: "settings",
        keys: &[KeyCode::Char('e')],
        upgrade_running: false,
        view: None,
    },
    State {
        name: "upgrade confirm",
        keys: &[KeyCode::Char('u'), KeyCode::Char('u')],
        upgrade_running: false,
        view: None,
    },
    State {
        name: "quit confirm",
        keys: &[KeyCode::Char('q')],
        upgrade_running: true,
        view: None,
    },
    State {
        name: "stats frame",
        keys: &[],
        upgrade_running: false,
        view: Some(views::stats),
    },
    State {
        name: "docker frame",
        keys: &[],
        upgrade_running: false,
        view: Some(views::docker),
    },
    State {
        name: "fetch frame",
        keys: &[],
        upgrade_running: false,
        view: Some(views::fetch),
    },
    State {
        name: "help over stats",
        keys: &[KeyCode::Char('?')],
        upgrade_running: false,
        view: Some(views::stats),
    },
];

/// The agent frames a panel shows, as the agent sends them.
mod views {
    use super::{Mode, Payload, Proc, Snapshot, TempUnit, Usage};

    pub fn stats() -> (Mode, Payload) {
        let snapshot = Snapshot {
            host: "srv-alpha (10.0.0.1)".into(),
            agent_version: "0.0.0".into(),
            cpu_pct: 42.0,
            cpu_mhz: Some(3600.0),
            proc_names: Vec::new(),
            cores: (0..8u32)
                .map(|c| {
                    (
                        c as usize,
                        f64::from(c).mul_add(12.0, 5.0),
                        Some(50.0 + f64::from(c)),
                    )
                })
                .collect(),
            temp_unit: TempUnit::default(),
            mem: Usage::new(16_000_000_000, 13_000_000_000),
            disk: Usage::new(500_000_000_000, 460_000_000_000),
            rx_rate: 50_000.0,
            tx_rate: 150_000.0,
            procs: (0..15)
                .map(|i| Proc {
                    pid: 2000 + i,
                    name: format!("svc-{i}"),
                    cpu: 5.0,
                    mem: 80_000_000,
                })
                .collect(),
        };
        (Mode::Monitor, Payload::Monitor(snapshot))
    }

    pub fn docker() -> (Mode, Payload) {
        let rows = (0..12u32)
            .map(|i| multitop_agent::docker::Row {
                name: format!("container-{i}"),
                status: if i % 3 == 2 {
                    "Exited (0)".into()
                } else {
                    "Up 3 hours".into()
                },
                image: format!("registry.example.com/team/image-{i}:latest"),
                cpu: format!("{:.1}%", f64::from(i) * 9.0),
                cpu_pct: f64::from(i) * 9.0,
                mem: format!("{}MiB / 512MiB", 64 + i * 32),
                mem_bytes: u64::from(64 + i * 32) << 20,
            })
            .collect();
        (
            Mode::Docker,
            Payload::Docker {
                host: "srv-alpha".into(),
                rows,
            },
        )
    }

    pub fn fetch() -> (Mode, Payload) {
        let snapshot = multitop_agent::fetch::FetchSnapshot {
            user_host: "admin@srv-alpha".into(),
            agent_version: "0.0.0".into(),
            os: "Debian GNU/Linux 13".into(),
            kernel: "6.12.0".into(),
            uptime: "12d 3h".into(),
            host_model: "Test Model".into(),
            cpu_model: "Test CPU (8)".into(),
            memory_str: "8.0GiB/16.0GiB (50%)".into(),
            disk_str: "200.0GiB/500.0GiB (40%)".into(),
        };
        (Mode::Fetch, Payload::Fetch(snapshot))
    }
}

/// Put the view's frame on panel 0 the way the event loop does: store the
/// payload, then render every panel at the pane's own size.
fn show(app: &mut App, view: fn() -> (Mode, Payload)) {
    let (mode, payload) = view();
    let panel = &mut app.panels[0];
    panel.mode = mode;
    match payload {
        Payload::Monitor(_) => panel.last_monitor = Some(payload),
        Payload::Docker { .. } => panel.last_docker = Some(payload),
        Payload::Fetch(snapshot) => panel.last_fetch = Some(snapshot),
        _ => unreachable!("a view this test does not build"),
    }
    let size = ratatui::layout::Size::new(SIZE.0, SIZE.1);
    let dims = multitop::ui::agent_dims(size, app.panels.len());
    app.rerender_all(dims);
}

fn server() -> Server {
    Server {
        user: "admin".to_string(),
        upgrade_cmd: Some("true".to_string()),
        ..common::local_server("srv-alpha")
    }
}

fn press(app: &mut App, code: KeyCode, tasks: &mut Tasks, tx: &mpsc::Sender<Msg>) {
    let (dims_tx, dims_rx) = watch::channel(SIZE);
    let dims_rx = Arc::new(dims_rx);
    handle_key(
        KeyEvent::new_with_kind(code, KeyModifiers::NONE, KeyEventKind::Press),
        app,
        SIZE,
        &dims_rx,
        tx,
        tasks,
    );
    drop(dims_tx);
}

fn resolve(colour: Color, default: Rgb) -> Result<Rgb, String> {
    match colour {
        Color::Reset => Ok(default),
        Color::Rgb(r, g, b) => Ok(Rgb::new(r, g, b)),
        other => Err(format!("{other:?}")),
    }
}

/// The row a failing glyph sits on, so a failure names the chrome it is in.
fn row_text(buf: &ratatui::buffer::Buffer, y: u16) -> String {
    (0..SIZE.0)
        .map(|x| buf[(x, y)].symbol())
        .collect::<String>()
        .trim()
        .to_string()
}

#[tokio::test]
async fn every_glyph_reads_against_the_background_it_is_drawn_on() {
    let _guard = password_store::lock_for_test_async().await;
    password_store::enable_mock_store();
    password_store::clear_mock_store();

    let mut failures = BTreeSet::new();
    let mut cells_measured = 0usize;
    for (theme_idx, theme) in THEMES.iter().enumerate() {
        for (terminal, background, painted, default_fg) in terminals() {
            let terminal_bg = painted.unwrap_or_else(|| theme.own_background());
            for state in STATES {
                let mut app = App::new(vec![server()]);
                app.theme_idx = theme_idx;
                app.background = background;
                app.rebuild_palette_view();
                if let Some(view) = state.view {
                    show(&mut app, view);
                }
                if state.upgrade_running {
                    app.panels[0].upgrade_state = UpgradeState::STARTED;
                }
                let (tx, _rx) = mpsc::channel::<Msg>(256);
                let mut tasks = Tasks::new(1);
                for &key in state.keys {
                    press(&mut app, key, &mut tasks, &tx);
                }
                let mut term = Terminal::new(TestBackend::new(SIZE.0, SIZE.1)).unwrap();
                term.draw(|f| multitop::ui::draw(f, &mut app)).unwrap();
                let buf = term.backend().buffer().clone();
                for y in 0..SIZE.1 {
                    for x in 0..SIZE.0 {
                        let cell = &buf[(x, y)];
                        if cell.symbol().trim().is_empty() {
                            continue;
                        }
                        cells_measured += 1;
                        let at = format!("{} / {terminal} / {}", theme.name, state.name);
                        let fg = resolve(cell.fg, default_fg);
                        let bg = resolve(cell.bg, terminal_bg);
                        let (fg, bg) = match (fg, bg) {
                            (Ok(fg), Ok(bg)) => (fg, bg),
                            (fg, bg) => {
                                let named = [fg.err(), bg.err()]
                                    .into_iter()
                                    .flatten()
                                    .collect::<Vec<_>>();
                                failures.insert(format!(
                                    "{at}: drawn in a terminal-decided colour, {}",
                                    named.join(" on ")
                                ));
                                continue;
                            }
                        };
                        let floor = if cell.modifier.contains(Modifier::BOLD) {
                            MIN_TEXT_CONTRAST
                        } else {
                            MIN_MUTED_CONTRAST
                        };
                        let ratio = fg.contrast_ratio(bg);
                        if ratio < floor {
                            failures.insert(format!(
                                "{at}, row {y}: {:?} fg {:?} on bg {:?} is {ratio:.2}:1, under {floor}",
                                row_text(&buf, y),
                                (fg.r, fg.g, fg.b),
                                (bg.r, bg.g, bg.b)
                            ));
                        }
                    }
                }
            }
        }
    }
    assert!(
        cells_measured > 10_000,
        "measured only {cells_measured} cells -- the scope moved"
    );
    assert!(
        failures.is_empty(),
        "{} illegible or terminal-decided colour pairs; the first 40:\n{}",
        failures.len(),
        failures
            .iter()
            .take(40)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
}
