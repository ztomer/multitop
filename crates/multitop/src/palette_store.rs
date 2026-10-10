//! The palette the app is drawing with, for text written where no `App` is in
//! reach -- a task's error line, a panel's placeholder.
//!
//! Those sites used to write `color::ANSI`: one theme's raw values, never
//! adapted, whatever theme was on and whatever the terminal painted. On Nord
//! with no answer from the terminal, "connecting..." came out at 2.65:1. There
//! is one terminal and one theme per process, so the palette is published
//! here by the one function that builds it, and read by everything that
//! cannot be handed it. A fixed palette outside this file is refused by
//! `tools/check_no_hard_coded_colour.py`.

use std::sync::{Arc, LazyLock, PoisonError, RwLock};

use multitop_agent::color::THEMES;
use multitop_agent::palette_view::PaletteView;

static CURRENT: LazyLock<RwLock<Arc<PaletteView>>> = LazyLock::new(|| RwLock::new(default_view()));

/// The palette before any background is known: the first theme, adapted
/// against the only background it states -- what `App::new` starts with.
#[must_use]
pub fn default_view() -> Arc<PaletteView> {
    Arc::new(PaletteView::for_background(
        &THEMES[0],
        THEMES[0].own_background(),
    ))
}

/// The palette the app last built.
#[must_use]
pub fn current() -> Arc<PaletteView> {
    Arc::clone(&CURRENT.read().unwrap_or_else(PoisonError::into_inner))
}

/// Make `view` the palette everything without an `App` draws with. Returns it,
/// so the builder can keep its own handle to the same value.
pub fn publish(view: Arc<PaletteView>) -> Arc<PaletteView> {
    *CURRENT.write().unwrap_or_else(PoisonError::into_inner) = Arc::clone(&view);
    view
}
