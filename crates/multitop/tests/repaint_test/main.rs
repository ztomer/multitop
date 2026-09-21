//! Where a repainting tool's output lands in the log.
//!
//! Split in two because the file grew past the length cap, and the seam was
//! already there: `painter` is the screen model on its own, with no app and no
//! async; `applying` drives real `Msg`s into a real `App` and asks what the
//! panel ends up holding. They fail for different reasons and are read at
//! different times.

// A test crate, said where clippy reads it: the restriction lints
// (`unwrap_used`, `expect_used`, `panic`) are policy for production code and
// exempt for test code (clippy.toml), and an integration test is test code
// through and through -- helpers included.
#![cfg(test)]

mod applying;
#[path = "../common/mod.rs"]
mod common;
mod painter;
