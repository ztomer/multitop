//! A palette that draws the adapted colours and still reads like a `Palette`.
//!
//! # Why this type exists rather than changing the 26 `&Palette` signatures
//!
//! The obvious way to wire [`adapt`] into the render path is to replace the
//! palette type everywhere it is passed. Measured, that is 26 signatures across
//! two crates, and the renderers read `pal.reset` and `pal.bold` as *fields* —
//! which is the part that makes it a refactor rather than a swap.
//!
//! But every one of those uses is either an argument to `format!` or a value
//! returned as `&'static str`. Neither needs a `'static` lifetime, because
//! nothing outlives the borrow. So a view with the same field and method names
//! satisfies all 26 signatures as they stand, and the refactor does not happen.
//!
//! Measured before deciding, which is the only reason this is a small change: the
//! first draft of the wiring assumed the 26 sites would each need editing, and
//! counting them properly cost ten minutes and saved an afternoon.
//!
//! # The cost of this shape
//!
//! The adapted escapes are owned, so a `PaletteView` is a bag of `String`s rather
//! than a `Copy` value. It must be **built once and held** — [`build`] is the only
//! constructor, and the App holds the result — because building it per frame
//! would allocate a dozen strings per frame, which is the exact cost the caching
//! in `App` exists to avoid. It is not `Copy` and does not pretend to be; the
//! `App` owns one and hands out borrows.

use crate::adapt::{adapt, slot, Adapted, Role, MIN_TEXT_CONTRAST};
use crate::color::Palette;
use crate::surface::Rgb;

/// The prefix of a 24-bit foreground colour escape.
///
/// Spelled here rather than composed from `surface`'s parser, because that parser
/// exists to READ a palette and this is the other direction. Both sides are
/// `38;2;R;G;B`, and a round-trip test in this module is what keeps them in step.
const ESCAPE_PREFIX: &str = "\x1b[38;2;";

/// The escape that closes a colour sequence.
const ESCAPE_SUFFIX: char = 'm';

/// A palette-shaped view over an [`Adapted`] palette.
///
/// The field and method names match [`Palette`] exactly, which is the entire
/// mechanism: the 26 places that take `&Palette` take `&PaletteView` instead and
/// their bodies do not change.
// The per-role fields are read by the renderers that take a palette — the 26
// `&Palette` signatures this type exists to satisfy — and by nothing here yet,
// because the view is not wired into `App::current_theme` yet. Clippy reports them
// as never read, and it is right: they are written and not yet consumed. Deleting
// them to silence that would delete the reason the type exists.
#[allow(dead_code)] // TODO: remove when App::current_theme returns a PaletteView.
pub struct PaletteView {
    /// One escape per role, in [`Role::all`] order, for the accessors.
    pub(crate) escapes: [String; slot::COUNT],
    /// `reset`, `bold` and `dim` are not colours and do not change with the
    /// background, so they are borrowed rather than rebuilt — the pointers are
    /// `Copy` and allocating a `String` per role per theme would not be.
    pub reset: &'static str,
    pub bold: &'static str,
    pub dim: &'static str,
    /// The roles the renderers read as FIELDS rather than through an accessor.
    /// `blue` is included because `ops::render` reaches for it, and it is given
    /// the mid-meter colour, which is the closest role to the blue in the
    /// palette's own naming.
    pub red: String,
    pub green: String,
    pub yellow: String,
    pub cyan: String,
    pub white: String,
    pub gray: String,
    pub purple: String,
    pub blue: String,
    /// The three colours the local TUI paints as `ratatui::Color` rather than as
    /// escapes: the keybar background, the border, and the accent.
    ///
    /// Colours like any other, so adapted the same way, and they are the ones a
    /// light terminal hurts most -- a keybar painted with a dark theme's
    /// background is a black bar across a white screen, which is more jarring
    /// than any text colour. Components rather than escapes, because ratatui
    /// wants components.
    pub keybar_bg: (u8, u8, u8),
    pub border: (u8, u8, u8),
    pub accent: (u8, u8, u8),
    /// The theme's name, for the config file. Never adapted: a name is not a
    /// colour and the config stores the theme, not a rendering of it.
    pub name: &'static str,
}

impl PaletteView {
    /// The view for `theme` on a terminal painting `background`.
    ///
    /// Falls back to the theme's own escapes when adaptation declines — a palette
    /// with no 24-bit colour, which is [`crate::color::PLAIN`] and a deliberate
    /// no-colour render. A blank palette would be worse than an unadapted one.
    #[must_use]
    pub fn for_background(theme: &Palette, background: Rgb) -> Self {
        let adapted = adapt(theme, background, MIN_TEXT_CONTRAST);
        Self::from_adapted(theme, adapted.as_ref())
    }

    /// The view for an already-computed adaptation.
    ///
    /// Separate from [`PaletteView::for_background`] so the App can adapt once
    /// and hold the result, rather than re-deriving it whenever a view is asked
    /// for.
    #[must_use]
    pub fn from_adapted(theme: &Palette, adapted: Option<&Adapted>) -> Self {
        let escape = |role: Role| -> String {
            adapted.map_or_else(
                || role.escape(theme).to_string(),
                |a| escape_for(a.get(role)),
            )
        };
        Self {
            escapes: std::array::from_fn(|index| escape(Role::from_index(index))),
            reset: theme.reset,
            bold: theme.bold,
            dim: theme.dim,
            red: escape(Role::MeterHigh),
            green: escape(Role::MeterLow),
            yellow: escape(Role::MeterMid),
            cyan: escape(Role::Primary),
            white: escape(Role::Text),
            gray: escape(Role::Muted),
            purple: escape(Role::Secondary),
            blue: escape(Role::MeterMid),
            keybar_bg: triple(theme.ratatui_keybar_bg, adapted, Role::Text),
            border: triple(theme.ratatui_border, adapted, Role::Primary),
            accent: triple(theme.ratatui_accent, adapted, Role::MeterLow),
            name: theme.name,
        }
    }

    /// Primary accent — headers, main metrics, titles, and the host banner.
    #[must_use]
    pub fn primary(&self) -> &str {
        &self.escapes[slot::PRIMARY]
    }

    /// Secondary accent — rules, subtitles, secondary metrics.
    #[must_use]
    pub fn secondary(&self) -> &str {
        &self.escapes[slot::SECONDARY]
    }

    /// Muted — PIDs, totals, indents.
    #[must_use]
    pub fn muted(&self) -> &str {
        &self.escapes[slot::MUTED]
    }

    /// Text — process and container names, primary values.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.escapes[slot::TEXT]
    }

    /// A meter reading low.
    #[must_use]
    pub fn meter_low(&self) -> &str {
        &self.escapes[slot::METER_LOW]
    }

    /// A meter reading medium.
    #[must_use]
    pub fn meter_mid(&self) -> &str {
        &self.escapes[slot::METER_MID]
    }

    /// A meter reading high, and the alert tint.
    #[must_use]
    pub fn meter_high(&self) -> &str {
        &self.escapes[slot::METER_HIGH]
    }

    /// The theme's name, for the config file.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        self.name
    }

    /// The colour for a container status, matching `Palette::status_color`'s
    /// classification exactly.
    ///
    /// Duplicated rather than delegated because the classification reads the
    /// palette's role accessors and the view's are the adapted escapes, so
    /// delegating would need a `Palette` this type does not have. A test below
    /// asserts the two agree on every status, because two copies of a
    /// classification is exactly the pair that drifts.
    #[must_use]
    pub fn status_color(&self, status: &str) -> &str {
        if status.starts_with("Up") || status == "running" {
            self.meter_low()
        } else if status.contains("Exited (0)") || status.contains("paused") {
            self.meter_mid()
        } else {
            self.meter_high()
        }
    }

    /// The colour for a CPU percentage, matching `Palette::cpu_bar`.
    #[must_use]
    pub fn cpu_bar(&self, pct: f64) -> &str {
        if pct >= crate::consts::CPU_HIGH_PCT {
            self.meter_high()
        } else if pct >= crate::consts::CPU_MID_PCT {
            self.meter_mid()
        } else {
            self.meter_low()
        }
    }

    /// The colour for a memory percentage, matching `Palette::mem_bar`.
    ///
    /// Note the low band is `primary`, not `meter_low`: memory that is fine is
    /// drawn in the theme's main colour, which is a palette decision rather than a
    /// typo, and the view has to reproduce it.
    #[must_use]
    pub fn mem_bar(&self, pct: f64) -> &str {
        if pct >= crate::consts::MEM_HIGH_PCT {
            self.meter_high()
        } else if pct >= crate::consts::MEM_MID_PCT {
            self.meter_mid()
        } else {
            self.primary()
        }
    }

    /// The colour for a disk percentage, matching `Palette::disk_bar`.
    #[must_use]
    pub fn disk_bar(&self, pct: f64) -> &str {
        if pct >= crate::consts::DISK_HIGH_PCT {
            self.meter_high()
        } else if pct >= crate::consts::DISK_MID_PCT {
            self.meter_mid()
        } else {
            self.meter_low()
        }
    }

    /// The view for `theme` on `theme`'s own stated background.
    ///
    /// The identity in every case, so it is what a test wants when it is checking
    /// layout or width rather than legibility: the escapes are the theme's own,
    /// byte for byte. Ten-odd test sites used to pass `&ANSI` and would otherwise
    /// each have to build a view, and a test that constructs its own subject is a
    /// test whose subject can drift from the one production passes.
    #[must_use]
    pub fn for_theme(theme: &Palette) -> Self {
        Self::for_background(theme, theme.own_background())
    }

    /// The escape array, for the test that checks the accessors index it the
    /// way `from_index` filled it.
    #[must_use]
    pub const fn escapes(&self) -> &[String; slot::COUNT] {
        &self.escapes
    }
}

/// The triple to use for a ratatui-painted colour.
///
/// The theme's own value when adaptation declined, and otherwise the nearest
/// ADAPTED ROLE's colour rather than an adapted version of this specific value.
/// A `PaletteView` maps roles onto a background; the three ratatui colours are
/// not roles, so the honest thing is to borrow the role each one visually tracks
/// -- the border is drawn in the primary, the accent in the low-meter green --
/// and take that role's adapted colour. Adapting a value that is not a role
/// would mean inventing a ninth role.
const fn triple(own: (u8, u8, u8), adapted: Option<&Adapted>, role: Role) -> (u8, u8, u8) {
    match adapted {
        None => own,
        Some(a) => {
            let c = a.get(role);
            (c.r, c.g, c.b)
        }
    }
}

/// The 24-bit escape for a colour, in the form a terminal will paint.
fn escape_for(colour: Rgb) -> String {
    let mut escape = String::with_capacity(ESCAPE_PREFIX.len() + CHANNEL_DIGITS);
    escape.push_str(ESCAPE_PREFIX);
    escape.push_str(&colour.r.to_string());
    escape.push(';');
    escape.push_str(&colour.g.to_string());
    escape.push(';');
    escape.push_str(&colour.b.to_string());
    escape.push(ESCAPE_SUFFIX);
    escape
}

/// The most digits a channel can print, `255`, so three.
const CHANNEL_DIGITS: usize = 3;

#[cfg(test)]
#[path = "palette_view_tests.rs"]
mod tests;
