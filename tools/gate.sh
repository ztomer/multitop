#!/usr/bin/env bash
# Per-repo gate entry point. Declares which toolchains this repo contains and
# delegates; it holds no gate logic of its own.
#   --staged : pre-commit scope (fast) — layer 1 only
#   --full   : pre-push scope — every layer
set -euo pipefail
GOH="${GOH_DIR:-${GOH:-$HOME/Projects/gates_of_heck}}"

"$GOH/gates/structural.sh" "$@"

case "${1:-}" in
  --full)
    # This is a Rust workspace.
    "$GOH/gates/rust_gate.sh" .
    # Layer 3: this repo's own checkers and its 95% coverage floor. The six
    # tools/check_*.py scripts and their self-tests; coverage comes through
    # tools/coverage_check.sh, the one definition of the floor that both CI
    # and the pre-commit hook call.
    ./tools/repo_gates.sh
    ;;
esac
