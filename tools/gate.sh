#!/usr/bin/env bash
# Per-repo gate entry point. Declares nothing of its own: the step list is
# GOH_CI_STEPS in .gatesrc and the checks live in gates_of_heck and tools/.
#   --staged : pre-commit scope (fast) -- structural gates over the index
#   --full   : pre-push scope -- every layer, via gates/local_ci.sh, under the
#              machine-wide gate lock (two full runs sharing the cargo target
#              dir serialize each other into a wall-clock that looks like a hang)
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
  *)
    exec "$GOH/gates/structural.sh" "$@"
    ;;
esac
