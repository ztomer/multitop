mod draw;
mod keybar;
mod keybar_confirm;
mod layout;
mod windowing;

pub use crate::refit::{refit_header, refit_line};
pub use draw::draw;
pub use keybar::{keybar_badges, keybar_content, keybar_line, mode_pair, FilterHint};
pub use layout::{agent_dims, regions, KEYBAR_H, MIN_AGENT_COLS, MIN_AGENT_ROWS};
pub use windowing::{pane_lines, visible, visible_upgrade};

/// A chrome colour as ratatui draws it. `None` is the terminal's own default,
/// which is what a palette that declines colour gets for every role.
#[must_use]
pub const fn ink(colour: Option<multitop_agent::surface::Rgb>) -> ratatui::style::Color {
    match colour {
        Some(c) => ratatui::style::Color::Rgb(c.r, c.g, c.b),
        None => ratatui::style::Color::Reset,
    }
}
