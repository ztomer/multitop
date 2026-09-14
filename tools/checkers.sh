#!/usr/bin/env bash
# Run every repo checker under tools/check_*.py: its --self-test first (a
# checker that cannot detect must not be trusted to report clean), then the
# check. The set is the GLOB, not a list -- a checker is run the moment the
# file exists, so nothing has to be added in three places. check_gate_parity
# runs first because it is the one that notices when the hook or CI fell
# behind the glob.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"
GOH="${GOH_DIR:-${GOH:-$HOME/Projects/gates_of_heck}}"
# shellcheck source=/Users/ztomer/Projects/gates_of_heck/tui/lib.sh
. "$GOH/tui/lib.sh"
log="$(mktemp "${TMPDIR:-/tmp}/multitop-checkers.XXXXXX")"
trap 'rm -f "$log"' EXIT
checkers=(tools/check_gate_parity.py)
for f in tools/check_*.py; do
    [ "$f" = tools/check_gate_parity.py ] || checkers+=("$f")
done
for c in "${checkers[@]}"; do
    name="$(basename "$c" .py)"
    if ! python3 "$c" --self-test >"$log" 2>&1; then
        cat "$log" >&2
        die "$name self-test failed -- it cannot be trusted to report clean"
    fi
    if ! python3 "$c" >"$log" 2>&1; then
        cat "$log" >&2
        die "$name failed"
    fi
    ok "$name"
done
