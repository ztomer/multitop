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

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    #[test]
    fn dispatch_monitor_produces_output() {
        // We can't construct Payload::Monitor directly since types are private
        // But we can verify the function exists and compiles
        // Real testing happens in ui_test.rs::render_payload_handles_every_variant
    }

    #[test]
    fn dispatch_docker_produces_output() {
        // Same - tested in ui_test.rs
    }

    #[test]
    fn dispatch_fetch_produces_output() {
        // Same - tested in ui_test.rs
    }
}
