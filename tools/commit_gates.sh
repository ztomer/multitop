#!/usr/bin/env bash
# This repo's own commit gates -- what a commit runs after gates_of_heck's
# structural gate. Reached ONLY through tools/gate.sh --staged, which the stock
# .githooks/pre-commit execs; gate.sh holds the per-clone gate lock around it.
#
#   the tools/check_*.py checkers (each behind its self-test), cargo fmt,
#   clippy -D warnings on all targets and features, the end-to-end suites, the
#   line-count ratchet, the fuzz targets compiling, and the 95% coverage floor.
#
# Until 2026-10-08 (O39) this was the second half of a customised
# .githooks/pre-commit, which gates_of_heck's install.sh refused to replace --
# so the hooks stayed on a retired generation. The hooks are now stock
# delegates install.sh owns; the gates moved here, unchanged in order and in
# what each refuses. check_gate_parity.py holds this file's checker list to
# CI's and to the glob, and checks that the hook -> gate.sh --staged -> this
# file chain is intact, since a list nothing runs agrees with CI perfectly.
#
# Clippy covers --all-targets because tests and benches carry the same lint
# config as src, and that is exactly where warnings have accumulated before.

set -euo pipefail

cd "$(git rev-parse --show-toplevel)"
GOH="${GOH_DIR:-${GOH:-$HOME/Projects/gates_of_heck}}"
# shellcheck source=/Users/ztomer/Projects/gates_of_heck/tui/lib.sh
. "$GOH/tui/lib.sh"
[ -f "$GOH/gates/_git_env.sh" ] || die "no gates/_git_env.sh in $GOH -- update gates_of_heck"

log="$(mktemp -t multitop-precommit)"
trap 'rm -f "$log"' EXIT

# in_index <cmd...> -- run a checker against the INDEX BEING COMMITTED. The
# stock hook drops git's repository variables before anything is spawned
# (gates_of_heck contract #12: a suite's scratch `git init` once wrote
# core.bare=true into the committing repo), and carries the index as
# GOH_HOOK_INDEX_FILE; `commit -a` and `commit <paths>` commit a temporary
# index that only that names. A checker's real run binds it again, so
# `git diff --cached` (check_doc_paths.py's staged deletions) sees this commit
# -- what the old hook's checkers saw. Its --self-test does NOT: a self-test
# builds fixtures, and a fixture must never inherit this repository's index.
in_index() {
    (
        # shellcheck source=/dev/null
        . "$GOH/gates/_git_env.sh"
        goh_bind_hook_index
        "$@"
    )
}

# Cargo may be missing from the environment a GUI git client hands us; say so
# plainly rather than failing with "command not found".
command -v cargo >/dev/null 2>&1 || die "cargo not on PATH — cannot run the fmt/clippy gates"
command -v python3 >/dev/null 2>&1 || die "python3 not on PATH — cannot run the emoji gate"

section "pre-commit gates"

# proven_step '<step>' -- run a step that GOH_CI_STEPS also runs, through
# gates_of_heck's proven-step cache (gates/proven.sh), output to $log. Spelled
# EXACTLY as GOH_CI_STEPS spells it; check_gate_parity.py holds the two to the
# letter, since a respelling would silently never hit. A pass on a clean tree
# (the working tree equal to the index being committed) is recorded, and the
# pre-push `tools/gate.sh --full` over the same tree skips that step instead of
# running it again. A partially staged commit has no key and records nothing.
# CI never reads the records: it stays the independent check. A gates_of_heck
# checkout that predates proven.sh runs the step uncached, and says so.
proven_step() {
    if [ -x "$GOH/gates/proven.sh" ]; then
        "$GOH/gates/proven.sh" --label pre-commit --log "$log" -- "$1"
    else
        warn "no gates/proven.sh in $GOH -- running uncached (update gates_of_heck)"
        bash -c "$1" >"$log" 2>&1 </dev/null
    fi
}

info "checking the gate lists agree"
if ! python3 tools/check_gate_parity.py --self-test >"$log" 2>&1; then
    cat "$log" >&2
    die "the gate-parity checker's own self-test failed -- it cannot be trusted"
fi
if ! in_index python3 tools/check_gate_parity.py >"$log" 2>&1; then
    cat "$log" >&2
    die "a gate is not run everywhere it should be"
fi
ok "hook, CI and local-ci run the same gates"

# Emoji: the house structural gate at the top of this hook already ran the
# shared checker over the index (gates_of_heck/checks/check_no_emoji.py,
# whose own tests cover escaped codepoints -- the padlock this repo once
# shipped as a unicode escape). The repo-local copy that used to run here
# was retired 2026-09-14; a copy is a checker that drifts.

info "checking for test-only code"
if ! python3 tools/check_test_only_code.py --self-test >"$log" 2>&1; then
    cat "$log" >&2
    die "the test-only checker's own self-test failed -- it cannot be trusted to report clean"
fi
if ! in_index python3 tools/check_test_only_code.py >"$log" 2>&1; then
    cat "$log" >&2
    die "a function is exercised only by tests — wire it up, delete it, or annotate it"
fi
ok "no new test-only code"

info "checking key hints"
if ! python3 tools/check_key_hints.py --self-test >"$log" 2>&1; then
    cat "$log" >&2
    die "the key-hint checker's own self-test failed -- it cannot be trusted to report clean"
fi
if ! in_index python3 tools/check_key_hints.py >"$log" 2>&1; then
    cat "$log" >&2
    die "a user-facing string names a key nothing binds"
fi
ok "key hints name live keys"

info "checking keychain isolation"
if ! python3 tools/check_keychain_isolation.py --self-test >"$log" 2>&1; then
    cat "$log" >&2
    die "the keychain-isolation checker's own self-test failed -- it cannot be trusted"
fi
if ! in_index python3 tools/check_keychain_isolation.py >"$log" 2>&1; then
    cat "$log" >&2
    die "a test can reach the real OS keychain — divert it with enable_mock_store"
fi
ok "tests cannot reach the real keychain"

info "checking row 0 has one owner"
if ! python3 tools/check_row0_owner.py --self-test >"$log" 2>&1; then
    cat "$log" >&2
    die "the row0-owner checker's own self-test failed -- it cannot be trusted"
fi
if ! in_index python3 tools/check_row0_owner.py >"$log" 2>&1; then
    cat "$log" >&2
    die "a pane's view is assigned outside panel.rs — use show_body or show_frame"
fi
ok "row 0 belongs to the banner alone"

info "checking the docs name paths that exist"
if ! python3 tools/check_doc_paths.py --self-test >"$log" 2>&1; then
    cat "$log" >&2
    die "the doc-paths checker's own self-test failed -- it cannot be trusted"
fi
if ! in_index python3 tools/check_doc_paths.py >"$log" 2>&1; then
    cat "$log" >&2
    die "a doc names a path that does not exist -- a reader following it runs nothing"
fi
ok "documented paths exist"

info "checking the render path asks the palette for colours"
if ! python3 tools/check_no_hard_coded_colour.py --self-test >"$log" 2>&1; then
    cat "$log" >&2
    die "the hard-coded-colour checker's own self-test failed -- it cannot be trusted"
fi
if ! in_index python3 tools/check_no_hard_coded_colour.py >"$log" 2>&1; then
    cat "$log" >&2
    die "a colour is hard-coded in the render path -- it will never be adapted"
fi
ok "render-path colours come from the palette"

info "checking for magic numbers"
if ! python3 tools/check_magic_numbers.py --self-test >"$log" 2>&1; then
    cat "$log" >&2
    die "the magic-number checker's own self-test failed -- it cannot be trusted"
fi
if ! in_index python3 tools/check_magic_numbers.py >"$log" 2>&1; then
    cat "$log" >&2
    die "a literal carries meaning nobody wrote down — give it a named constant"
fi
ok "no unnamed literals"

info "checking agent version matches embedded binary"
if ! python3 tools/check_agent_version.py --self-test >"$log" 2>&1; then
    cat "$log" >&2
    die "the agent-version checker's own self-test failed"
fi
if ! in_index python3 tools/check_agent_version.py >"$log" 2>&1; then
    cat "$log" >&2
    die "embedded agent stale — rebuild with ./build.sh"
fi
ok "agent version matches"

info "checking codesign is stable"
if ! python3 tools/check_codesign.py --self-test >"$log" 2>&1; then
    cat "$log" >&2
    die "the codesign checker's own self-test failed"
fi
if ! in_index python3 tools/check_codesign.py >"$log" 2>&1; then
    cat "$log" >&2
    die "binary signature unstable — run: for f in ~/.cargo/bin/multitop target/debug/multitop target/release/multitop; do codesign -s - --identifier com.ztomer.multitop \"\$f\"; done"
fi
ok "codesign stable"

info "checking unsafe stays inside the FFI modules"
if ! python3 tools/check_unsafe_scope.py --self-test >"$log" 2>&1; then
    cat "$log" >&2
    die "the unsafe-scope checker's own self-test failed"
fi
if ! in_index python3 tools/check_unsafe_scope.py >"$log" 2>&1; then
    cat "$log" >&2
    die "unsafe outside the FFI allowlist — see tools/check_unsafe_scope.py"
fi
ok "unsafe scope"

info "checking formatting"
if ! cargo fmt --all -- --check >"$log" 2>&1; then
    cat "$log" >&2
    die "formatting is dirty — run: cargo fmt --all"
fi
ok "formatting clean"

info "running clippy (all targets, all features, -D warnings)"
if ! cargo clippy --workspace --all-targets --all-features -- -D warnings >"$log" 2>&1; then
    tail -60 "$log" >&2
    die "clippy is not clean — fix the warnings, do not add #[allow]"
fi
ok "clippy clean"

# Through the script, not a second copy of the command. This hook used to run
# its own `--fail-under-lines 55` under a label that said 95, against one crate
# and without the documented exclusions -- a local gate weaker than CI is a gate
# that lets you push red.
info "running the end-to-end suites (tests/test_exec_live.py, tests/test_tmux_e2e.py)"
# These drive the built binary, so the gate builds it. An e2e suite run against
# a stale binary tests code nobody wrote -- the harness refuses to, and would
# fail here rather than quietly passing.
#
# Both skip with a stated reason when what they need is absent: the live suite
# without reachable hosts, the tmux suite without tmux. Neither passes silently.
if ! cargo build -p multitop >"$log" 2>&1; then
    tail -20 "$log" >&2
    die "cannot build multitop, so the end-to-end suites cannot run"
fi
if ! python3 -m pytest tests/ -q >"$log" 2>&1; then
    tail -40 "$log" >&2
    die "an end-to-end suite failed"
fi
ok "end-to-end suites"

info "checking the line-count ratchet"
# Was local-ci-only, under a comment saying it is "a shape rule rather than a
# correctness one". True, and it still went red on a commit this hook passed --
# so the shape rule was enforced only by whoever remembered to run the pre-push
# script. It costs a glob and a line count.
#
# Through the proven-step cache (proven_step, above), spelled as GOH_CI_STEPS
# spells it -- self-test included -- so the push-time run is a hit.
if ! proven_step 'python3 tools/ratchet_check.py --self-test && python3 tools/ratchet_check.py'; then
    cat "$log" >&2
    die "a file grew past its recorded ceiling -- shrink it, or re-record in this commit"
fi
ok "line-count ratchet"

info "checking the fuzz targets still compile"
# `cargo check`, not `cargo fuzz build`: the full build is six sanitizer release
# binaries on a nightly toolchain, which is minutes, and a hook that costs
# minutes is a hook people learn to --no-verify past. The failure this actually
# catches is a target that stopped compiling because what it fuzzes changed
# shape -- `fuzz_proto` went non-exhaustive when the protocol gained a variant,
# and nothing said so until a release. That is a compile error, and this finds
# it in seconds. CI keeps the full sanitizer build.
if ! cargo check --manifest-path fuzz/Cargo.toml --all-targets >"$log" 2>&1; then
    tail -30 "$log" >&2
    die "a fuzz target no longer compiles against the code it fuzzes"
fi
ok "fuzz targets compile"

info "checking coverage (workspace, 95% floor)"
# Through the proven-step cache (proven_step, above): a pass here is what lets
# the pre-push run skip the identical ~10 min suite over the same tree.
if ! proven_step 'bash tools/coverage_check.sh'; then
    tail -20 "$log" >&2
    die "coverage below 95% — add tests"
fi
ok "coverage >= 95%"

printf '\n'
