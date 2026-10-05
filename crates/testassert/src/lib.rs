//! The two assertions this workspace uses instead of asserting emptiness by hand.
//!
//! # Why these exist
//!
//! Three gates, in two tools, all refusing the same line — and the second and
//! third only started refusing it in October 2026, which is why 115 of them
//! were sitting in a tree whose every other gate was green:
//!
//! * `clippy::assert_is_empty` (pedantic, new in clippy 1.99.0) fires on
//!   `assert!(v.is_empty())` and `assert!(!v.is_empty())`.
//! * `clippy::len_zero` fires on `assert!(v.len() == 0)`.
//! * `gates_of_heck/checks/check_no_empty_assert.py` fires on BOTH of those
//!   **and** on the same shapes carrying a message — which clippy is silent
//!   about, measured, so clippy alone reports 41 of the 115 and the house
//!   checker reports all of them.
//!
//! The lint's actual objection is a diagnosability one, and that is what these
//! answer. `assert!(v.is_empty())` fails with `assertion failed: is_empty()`,
//! which says neither which value nor what was in it; clippy's own suggestion
//! — `assert_eq!(v, [] as [T; 0])` — fixes that by printing the value, and
//! costs a type ascription at every one of 115 sites. Both are the wrong shape.
//!
//! These name the offending expression on failure, and require no trait of the
//! value at all -- no `Debug`, no `len()`. That is a decision with a compiler
//! behind it, from two receivers in this workspace that are the reason:
//!
//! * `crates/vault/src/mlock.rs`'s `LockedMemory`, the mlocked copy of a vault
//!   key, has **no** `Debug` implementation, precisely so key bytes cannot
//!   reach a log. A helper that interpolated `{value:?}` does not compile
//!   there, and the fix anyone reaches for under `-D warnings` is to add
//!   `Debug` to the one type that must not have it.
//! * `ratatui::prelude::Modifier` (`crates/multitop/src/ansi.rs`) has
//!   `is_empty()` and **no** `len()` -- it is a bitflags set. A helper that
//!   counted elements does not compile there, and the fix anyone reaches for
//!   is to add a `len()` to a foreign bitflags type.
//!
//! Both errors were found by compiling, not by reading. The property that
//! matters is that `assert_empty!(anything)` always works: a helper that can
//! fail to compile at a new call site is not the path of least resistance, and
//! an assertion nobody reaches for is not an assertion.
//!
//! So the message carries the EXPRESSION, which is what identifies which of a
//! dozen similar assertions failed -- strictly more than
//! `assertion failed: is_empty()` said, with no bound at all:
//!
//! ```ignore
//! multitop_testassert::assert_not_empty!(snap.agent_version);
//! // expected not empty: snap.agent_version
//! ```
//!
//! A test that wants to say more passes it as the optional `why`, which is
//! appended verbatim.
//!
//! # Why the bodies are written the way they are
//!
//! Each body asserts emptiness the shape the house checker EXEMPTS: an
//! interpolated message. From that checker's own docstring -- *"a message that
//! interpolates the value is also exempt, because it is the improvement this
//! gate exists to ask for, already made"* -- so this crate is written in the
//! vocabulary the gate sanctions rather than in a shape it tolerates, and it
//! needs no exemption, no `#[allow]` and no `#[expect]` anywhere.
//!
//! A `panic!` here would have been the obvious alternative and is the wrong
//! one twice over: the `panic` lint is on workspace-wide, and a `panic!`
//! inside a `macro_rules!` body reports the macro's own span, so the failure
//! would name this file instead of the assertion.
//!
//! # Why a crate, and not a module
//!
//! One definition, reachable from all three shipped crates' tests -- including
//! the `#[cfg(test)]` sites inside `src/`, which a `tests/common/` module
//! cannot see. `vault` is the case that forces it: its only two sites are in
//! `src/mlock.rs`, and it shares no dependency with the other two crates, so
//! there is no existing crate to hang a helper off. Being a dev-dependency of
//! all three, it never reaches a shipped binary -- it is not linked into
//! `multitop`, and the size-tuned agent that gets uploaded to every monitored
//! host does not contain a byte of it.

/// Assert that `$value` is empty, naming it on failure.
///
/// The empty case. An optional second argument is a static `why`, appended to
/// the failure message for intent the expression itself cannot carry:
///
/// ```ignore
/// multitop_testassert::assert_empty!(parse_container_list(""));
/// multitop_testassert::assert_empty!(stat.cores, "a CPU-less host reports no cores");
/// ```
#[macro_export]
macro_rules! assert_empty {
    ($value:expr $(,)?) => {{
        let value = &$value;
        assert!(value.is_empty(), "expected empty: {}", stringify!($value));
    }};
    ($value:expr, $why:expr $(,)?) => {{
        let value = &$value;
        assert!(
            value.is_empty(),
            "expected empty: {}: {}",
            stringify!($value),
            $why
        );
    }};
}

/// Assert that `$value` is **not** empty, naming it on failure.
///
/// The negated case, spelled so there is no `!` to misread. `assert_ne!(v.len(),
/// 0)` is clippy's own suggestion and passes both gates, but it cannot carry
/// the expression's name -- and in a file with a dozen emptiness assertions,
/// the expression is most of what the reader needs.
///
/// ```ignore
/// multitop_testassert::assert_not_empty!(snap.agent_version);
/// ```
#[macro_export]
macro_rules! assert_not_empty {
    ($value:expr $(,)?) => {{
        let value = &$value;
        assert!(
            !value.is_empty(),
            "expected not empty: {}",
            stringify!($value)
        );
    }};
    ($value:expr, $why:expr $(,)?) => {{
        let value = &$value;
        assert!(
            !value.is_empty(),
            "expected not empty: {}: {}",
            stringify!($value),
            $why
        );
    }};
}

#[cfg(test)]
mod tests {
    /// Run `$body`, require it to fail, and hand back the failure line.
    ///
    /// The failure text is the entire product of these macros -- the lint they
    /// replace objected to an assertion that named nothing -- so it is asserted
    /// rather than assumed. A local macro rather than a function taking the
    /// closure, because clippy's `needless_pass_by_value` fires on an
    /// `impl FnOnce()` parameter however it is consumed (measured 2026-10-04,
    /// clippy 1.99.0): it cannot see the move through `catch_unwind`'s generic
    /// call, so a function here fails the gate over its own test helper.
    macro_rules! failure {
        ($body:expr) => {{
            let payload =
                std::panic::catch_unwind($body).expect_err("this assertion must have failed");
            payload
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_owned()))
                .expect("a panic payload is a String or a &'static str")
        }};
    }

    /// The empty case, and the one thing the failure has to say.
    #[test]
    fn assert_empty_passes_on_empty_and_names_the_expression() {
        crate::assert_empty!(Vec::<u8>::new());
        crate::assert_empty!(String::new());
        let message = failure!(|| crate::assert_empty!([1u8, 2]));
        assert!(
            message.contains("[1u8, 2]"),
            "the EXPRESSION is what identifies which of a dozen assertions \
             failed, so it has to be in the text: {message}"
        );
    }

    /// The negated case, spelled without the `!` a reader has to parse.
    #[test]
    fn assert_not_empty_passes_on_present_and_names_the_expression() {
        crate::assert_not_empty!([1u8]);
        let message = failure!(|| crate::assert_not_empty!(Vec::<u8>::new()));
        assert!(message.contains("Vec::<u8>::new()"), "{message}");
    }

    /// The optional `why` reaches the failure line.
    #[test]
    fn a_why_is_appended() {
        let message = failure!(|| {
            crate::assert_empty!([1u8], "a CPU-less host reports no cores");
        });
        assert!(
            message.ends_with("a CPU-less host reports no cores"),
            "{message}"
        );
        let message = failure!(|| {
            crate::assert_not_empty!(Vec::<u8>::new(), "silence is the defect");
        });
        assert!(message.ends_with("silence is the defect"), "{message}");
    }

    /// Values with `is_empty` and deliberately **no** `Debug` and **no**
    /// `len()`, because that is what the two real receivers are.
    ///
    /// `vault`'s `LockedMemory` is the mlocked copy of a vault key, without
    /// `Debug` so key bytes cannot reach a log. `ratatui`'s `Modifier` is a
    /// bitflags set with `is_empty()` and no `len()`. Both errors were found
    /// by compiling a macro that asked for one trait or the other; a
    /// regression test that does not reproduce them is a test whose next
    /// failure gets "fixed" by adding `Debug` to the one type in the
    /// workspace that must not have it.
    struct NeitherTrait(&'static str);

    impl NeitherTrait {
        fn is_empty(&self) -> bool {
            self.0.is_empty()
        }
    }

    #[test]
    fn a_value_with_neither_trait_asserts_all_the_same() {
        crate::assert_empty!(NeitherTrait(""));
        crate::assert_not_empty!(NeitherTrait("x"));
        let message = failure!(|| crate::assert_empty!(NeitherTrait("x")));
        assert!(message.contains("NeitherTrait(\"x\")"), "{message}");
    }
}
