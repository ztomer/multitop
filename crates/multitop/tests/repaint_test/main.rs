//! Where a repainting tool's output lands in the log.
//!
//! Split in two because the file grew past the length cap, and the seam was
//! already there: `painter` is the screen model on its own, with no app and no
//! async; `applying` drives real `Msg`s into a real `App` and asks what the
//! panel ends up holding. They fail for different reasons and are read at
//! different times.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod applying;
mod painter;
