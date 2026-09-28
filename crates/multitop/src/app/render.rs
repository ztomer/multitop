//! View rendering and theme cycling.

use crate::app::{App, VaultState};
use crate::panel::Mode;

impl App {
    pub fn cycle_theme(&mut self) {
        self.theme_idx = (self.theme_idx + 1) % multitop_agent::color::THEMES.len();
        self.rebuild_palette_view();
    }

    /// The palette the renderers draw with, adapted for the detected background.
    ///
    /// The theme's own NAME is still on the `App` for the config file, and
    /// `current_theme_name` is what the two theme-`save` sites use — a config that
    /// recorded a rendered colour instead of a theme would be unreadable by the
    /// next version of itself.
    #[must_use]
    pub fn current_theme(&self) -> std::sync::Arc<multitop_agent::palette_view::PaletteView> {
        std::sync::Arc::clone(&self.palette_view)
    }

    /// The selected theme's name, for the config file.
    ///
    /// Separate from [`App::current_theme`] on purpose: the view carries the
    /// adapted escapes, and what gets written to disk has to be the theme a human
    /// chose.
    #[must_use]
    pub const fn current_theme_name(&self) -> &'static str {
        multitop_agent::color::THEMES[self.theme_idx].name
    }

    /// Re-derive the adapted palette for the current theme and background.
    ///
    /// Called wherever the theme or the background can have changed, and nowhere
    /// else — this allocates, and a render path that allocated per frame would be
    /// a worse bug than the one it fixes.
    pub fn rebuild_palette_view(&mut self) {
        let theme = &multitop_agent::color::THEMES[self.theme_idx];
        let background = self.background.or_theme(theme.own_background());
        self.palette_view = std::sync::Arc::new(
            multitop_agent::palette_view::PaletteView::for_background(theme, background),
        );
    }

    pub fn rerender_all(&mut self, dims: (u16, u16)) {
        let pal = self.current_theme();
        let pal = pal.as_ref();
        let sort = self.sort;
        let vault_locked = self.vault.is_some() && matches!(self.vault_state, VaultState::Locked);
        for panel in &mut self.panels {
            match panel.mode {
                Mode::Monitor => {
                    if let Some(payload) = &panel.last_monitor {
                        let lines = crate::render_payload::render_payload(payload, dims, sort, pal);
                        panel.show_frame(lines);
                    }
                }
                Mode::Alerts => {
                    let lines = crate::graphs::render_alerts(
                        &panel.history,
                        dims.0 as usize,
                        dims.1 as usize,
                        pal,
                        crate::graphs::AlertConfig {
                            cpu: self.alert_cpu,
                            mem: self.alert_mem,
                            disk: self.alert_disk,
                            vault_locked,
                        },
                    );
                    panel.show_frame(lines);
                }
                Mode::Graphs => {
                    // A resize changes how many samples fit, so the graph is
                    // redrawn from the history rather than refitted -- refitting
                    // would stretch braille cells into nonsense.
                    let lines = crate::graphs::render_graphs_with_zoom(
                        &panel.history,
                        dims.0 as usize,
                        dims.1 as usize,
                        pal,
                        self.graph_zoom,
                    );
                    panel.show_frame(lines);
                }
                Mode::Docker => {
                    if let Some(payload) = &panel.last_docker {
                        let lines = crate::render_payload::render_payload(payload, dims, sort, pal);
                        panel.show_frame(lines);
                    }
                }
                Mode::Fetch => {
                    if let Some(snap) = &panel.last_fetch {
                        let lines = crate::fetch_render::render_fetch(
                            snap,
                            dims.0 as usize,
                            dims.1 as usize,
                            pal,
                        );
                        panel.show_frame(lines);
                    }
                }
                Mode::Ops => {
                    let lines = crate::app::ops_view::frame(panel, dims, pal);
                    panel.show_frame(lines);
                }
                Mode::Upgrade => {}
            }
        }
    }
}
