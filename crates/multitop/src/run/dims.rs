//! Agent render size computation and publishing.
//!
//! The agent needs to know how big a pane is so it can pre-render frames.
//! `AgentDims` computes that from the terminal size and panel count, and
//! publishes it through a watch channel.

use tokio::sync::watch;

use crate::ui;

/// Everything the agent render size is derived from.
///
/// One value rather than two, and diffed whole. The size used to be recomputed
/// only when a `Resize` arrived, from a panel count captured before the first
/// frame -- so editing the server list changed the grid on screen (one column
/// becomes two at three panels) while every agent kept rendering for the old
/// one. A per-input hook misses whichever input it was not written for, and
/// this is the input it missed.
/// Recompute the agent render size from a terminal size query, or keep the last.
///
/// # Why the policy for a failed query lives in one function
///
/// It lived in three places with two different answers. Two of them wrote
/// `terminal.size().ok().and_then(...)` -- keep the last known size and carry on
/// -- under a comment stating the reasoning outright: *"a terminal that cannot
/// report its size is not a terminal of size zero, and treating it as one
/// publishes the minimum render size to every agent and re-renders the whole
/// grid tiny. Keeping the last known size is the honest failure."*
///
/// The third, the resize arm, made the same failure **fatal**: it set the loop's
/// error and broke, and every exit from that loop runs `abort_all`. So a
/// transient `ioctl` failure while the user dragged a window corner ended the
/// session *and killed every upgrade in flight*, leaving `dpkg` half-done and a
/// lock file behind on each host -- for a condition its sibling five hundred
/// lines up documents as survivable.
///
/// It is also redundant as a way of noticing the terminal is gone: that arrives
/// as `Some(Err(_)) | None` from the event stream and is already an orderly quit.
#[allow(
    clippy::needless_pass_by_value,
    reason = "takes the backend's own Result; E is generic and may not be Copy"
)]
pub(super) fn size_change<E>(
    query: Result<ratatui::layout::Size, E>,
    dims: &mut AgentDims,
    panels: usize,
) -> Option<(u16, u16)> {
    query.map_or(None, |size| dims.refresh(size, panels))
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct DimsInputs {
    size: (u16, u16),
    panels: usize,
}

/// The agent render size, and the only thing allowed to publish it.
pub(super) struct AgentDims {
    /// `None` until a size query has succeeded, so the first one that does is
    /// never mistaken for "nothing changed".
    inputs: Option<DimsInputs>,
    dims: (u16, u16),
    tx: watch::Sender<(u16, u16)>,
}

impl AgentDims {
    /// Seed from a size query that may have failed.
    ///
    /// `terminal.size().unwrap_or_default()` was the third policy for this one
    /// call, and `size_change`'s own comment names it as the wrong one: a size
    /// of zero yields the *minimum* agent render size, so a transient failure
    /// here would have published 40x4 to every agent and rendered the whole grid
    /// tiny, with nothing to recover it until the next resize.
    ///
    /// There is nothing to fall back to on failure but the value the caller
    /// already published, which is what the channel holds -- so that is what is
    /// kept, and `inputs` is left unmeasured so the next successful query
    /// recomputes rather than comparing against a size nobody ever read.
    #[allow(
        clippy::needless_pass_by_value,
        reason = "takes the backend's own Result; E is generic and may not be Copy"
    )]
    pub(super) fn new<E>(
        tx: watch::Sender<(u16, u16)>,
        query: Result<ratatui::layout::Size, E>,
        panels: usize,
    ) -> Self {
        let Ok(size) = query else {
            let dims = *tx.borrow();
            return Self {
                inputs: None,
                dims,
                tx,
            };
        };
        let inputs = Some(DimsInputs {
            size: (size.width, size.height),
            panels,
        });
        let dims = ui::agent_dims(size, panels);
        let _ = tx.send(dims);
        Self { inputs, dims, tx }
    }

    /// The terminal size this was last measured at, if it ever was.
    pub(super) fn last_size(&self) -> Option<(u16, u16)> {
        self.inputs.map(|i| i.size)
    }

    pub(super) const fn current(&self) -> (u16, u16) {
        self.dims
    }

    /// Recompute from the current inputs. Returns the new size when anything
    /// that feeds it changed, so the caller can re-render at it.
    pub(super) fn refresh(
        &mut self,
        size: ratatui::layout::Size,
        panels: usize,
    ) -> Option<(u16, u16)> {
        let inputs = Some(DimsInputs {
            size: (size.width, size.height),
            panels,
        });
        if inputs == self.inputs {
            return None;
        }
        self.inputs = inputs;
        let dims = ui::agent_dims(size, panels);
        if dims == self.dims {
            return None;
        }
        self.dims = dims;
        let _ = self.tx.send(dims);
        Some(dims)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use ratatui::layout::Size;

    fn size(w: u16, h: u16) -> Size {
        Size {
            width: w,
            height: h,
        }
    }

    /// A terminal that cannot report its size is not a terminal of size zero:
    /// treating it as one publishes the minimum render size to every agent and
    /// draws the whole grid tiny.
    #[test]
    fn a_failed_first_query_keeps_what_was_already_published() {
        let (tx, rx) = watch::channel((100, 20));
        let dims = AgentDims::new::<std::io::Error>(tx, Err(std::io::Error::other("no ioctl")), 3);

        assert_eq!(dims.current(), (100, 20), "the published size was replaced");
        assert_eq!(*rx.borrow(), (100, 20), "a value was published on failure");
        assert_eq!(
            dims.last_size(),
            None,
            "an unmeasured terminal must not claim a size, or the next \
             successful query compares against one nobody read"
        );
    }

    #[test]
    fn a_successful_first_query_publishes_and_records_it() {
        let (tx, rx) = watch::channel((0, 0));
        let dims = AgentDims::new::<std::io::Error>(tx, Ok(size(120, 40)), 2);

        assert_eq!(dims.last_size(), Some((120, 40)));
        assert_eq!(dims.current(), ui::agent_dims(size(120, 40), 2));
        assert_eq!(*rx.borrow(), dims.current());
    }

    #[test]
    fn recomputing_from_unchanged_inputs_publishes_nothing() {
        let (tx, mut rx) = watch::channel((0, 0));
        let mut dims = AgentDims::new::<std::io::Error>(tx, Ok(size(120, 40)), 2);
        rx.borrow_and_update();

        assert_eq!(dims.refresh(size(120, 40), 2), None);
        assert!(
            !rx.has_changed().unwrap(),
            "an unchanged size was republished"
        );
    }

    /// Two different inputs can still yield the same render size — the grid
    /// quantises. Nothing changed on screen, so nothing is published.
    #[test]
    fn inputs_that_change_without_changing_the_render_size_publish_nothing() {
        let (tx, mut rx) = watch::channel((0, 0));
        let mut dims = AgentDims::new::<std::io::Error>(tx, Ok(size(120, 40)), 2);
        rx.borrow_and_update();

        // The grid quantises in principle, so a different terminal size can
        // land on the same agent size and must not be republished. Searched
        // rather than hardcoded, and skipped when this build's mapping happens
        // to be injective over the range -- the assertion below is the point,
        // not the search.
        let same = (1..24)
            .flat_map(|dw| (0..24).map(move |dh| (120 + dw, 40 + dh)))
            .find(|&(w, h)| ui::agent_dims(size(w, h), 2) == dims.current());
        if let Some((w, h)) = same {
            assert_eq!(
                dims.refresh(size(w, h), 2),
                None,
                "a size that draws identically was republished"
            );
            assert!(!rx.has_changed().unwrap());
        }
    }

    /// The panel count feeds the size as much as the terminal does: three
    /// panels are two columns where two are one.
    #[test]
    fn a_changed_panel_count_republishes_without_any_resize() {
        let (tx, rx) = watch::channel((0, 0));
        let mut dims = AgentDims::new::<std::io::Error>(tx, Ok(size(120, 40)), 3);
        let before = dims.current();

        let after = dims.refresh(size(120, 40), 2);
        assert_eq!(after, Some(ui::agent_dims(size(120, 40), 2)));
        assert_ne!(after.unwrap(), before);
        assert_eq!(*rx.borrow(), after.unwrap());
    }

    /// A transient `ioctl` failure while the user drags a window corner used
    /// to end the session and kill every upgrade in flight.
    #[test]
    fn a_failed_query_mid_session_is_survivable_rather_than_fatal() {
        let (tx, _rx) = watch::channel((0, 0));
        let mut dims = AgentDims::new::<std::io::Error>(tx, Ok(size(120, 40)), 3);
        let before = dims.current();

        assert_eq!(
            size_change(Err(std::io::Error::other("ioctl failed")), &mut dims, 3),
            None
        );
        assert_eq!(dims.current(), before, "the last known size was discarded");

        // And a successful query after it still recomputes.
        assert!(size_change::<std::io::Error>(Ok(size(80, 24)), &mut dims, 3).is_some());
    }
}
