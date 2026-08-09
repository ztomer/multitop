//! Render payload dispatcher for monitor, docker, and fetch payloads.

use multitop_agent::color::Palette;
use multitop_agent::proto::Payload;
use multitop_agent::render::{bar_len_for, render};
use multitop_agent::SortBy;

/// Dispatch a received packet to the correct renderer at the given dimensions.
///
/// Extracted so the resize → re-render path can be tested without SSH.
#[must_use]
pub fn render_payload(
    payload: &Payload,
    dims: (u16, u16),
    sort: SortBy,
    pal: &Palette,
) -> Vec<String> {
    let (cols, height) = dims;
    let bar_len = bar_len_for(cols as usize);
    match payload {
        Payload::Monitor(snap) => render(snap, cols as usize, height as usize, bar_len, pal),
        Payload::Docker { host, rows } => {
            multitop_agent::docker::render(host, cols as usize, height as usize, rows, pal, sort)
        }
        Payload::Fetch(snap) => {
            crate::fetch_render::render_fetch(snap, cols as usize, height as usize, pal)
        }
    }
}
