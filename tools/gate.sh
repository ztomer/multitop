#!/usr/bin/env bash
# Per-repo gate entry point, and the only one the hooks call: the stock
# .githooks/pre-commit execs `--staged`, the stock pre-push runs `--full` on
# the pushed commit in a clean worktree (gates_of_heck/gates/push_gate.sh).
# The checks live in gates_of_heck and tools/; the lists are named once each.
#   --staged : pre-commit scope -- the structural gates over the index, then
#              this repo's commit gates (tools/commit_gates.sh)
#   --full   : pre-push scope -- every layer: GOH_CI_STEPS in .gatesrc, via
#              gates/local_ci.sh
# Both run under tools/gate_lock.py, one gate run per clone: a second suite
# alongside this one contends the cargo build dir and the CPU until
# timing-sensitive suites flake. The owner pid is this shell's, captured once
# (the helper is short-lived, so its own pid would read stale the instant it
# exits), and the trap releases it on every exit path.
set -euo pipefail
GOH="${GOH_DIR:-${GOH:-$HOME/Projects/gates_of_heck}}"
root="$(git rev-parse --show-toplevel)"
case "${1:-}" in
  --full)
    cd "$root"
    python3 tools/gate_lock.py acquire "$$"
    trap 'python3 tools/gate_lock.py release "$$"' EXIT
    "$GOH/gates/local_ci.sh" "$root"
    ;;
  --staged)
    cd "$root"
    python3 tools/gate_lock.py acquire "$$"
    trap 'python3 tools/gate_lock.py release "$$"' EXIT
    "$GOH/gates/structural.sh" --staged
    bash tools/commit_gates.sh
    ;;
  *)
    exec "$GOH/gates/structural.sh" "$@"
    ;;
esac
