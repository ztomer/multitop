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
