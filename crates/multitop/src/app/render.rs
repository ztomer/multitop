//! View rendering and theme cycling.

use crate::app::{App, VaultState};
use crate::panel::Mode;

impl App {
    pub fn cycle_theme(&mut self) {
        self.theme_idx = (self.theme_idx + 1) % multitop_agent::color::THEMES.len();
    }

    #[must_use]
    pub fn current_theme(&self) -> &'static multitop_agent::color::Palette {
        &multitop_agent::color::THEMES[self.theme_idx]
    }

    pub fn rerender_all(&mut self, dims: (u16, u16)) {
        let pal = self.current_theme();
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
                Mode::Upgrade => {}
            }
        }
    }
}
