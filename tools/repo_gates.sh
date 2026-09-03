#!/usr/bin/env bash
# This repo's own gates, as layer 3 of tools/gate.sh.
#
# The six checkers in tools/ run everywhere -- hook, CI and scripts/local-ci.py
# agree on the list (tools/check_gate_parity.py enforces that); this script is
# where tools/gate.sh picks them up for its full run. Each behind its
# self-test: a checker that has quietly stopped detecting reports a clean tree
# exactly like a clean tree does.
#
# Coverage goes through tools/coverage_check.sh, not a second copy of the
# command, so this gate cannot be weaker than CI's.
set -euo pipefail
repo_root="$(git rev-parse --show-toplevel)"
cd "$repo_root"
. "$repo_root/tui/lib.sh"

log="$(mktemp -t multitop-repo-gates)"
trap 'rm -f "$log"' EXIT

section "repo gates"

for checker in \
    check_gate_parity \
    check_no_emoji \
    check_test_only_code \
    check_key_hints \
    check_keychain_isolation \
    check_row0_owner \
    check_magic_numbers \
    check_agent_version \
    check_codesign
do
    info "$checker"
    if ! python3 "tools/$checker.py" --self-test >"$log" 2>&1; then
        cat "$log" >&2
        die "$checker self-test failed -- it cannot be trusted to report clean"
    fi
    if ! python3 "tools/$checker.py" >"$log" 2>&1; then
        cat "$log" >&2
        die "$checker failed"
    fi
    ok "$checker"
done

info "coverage (workspace, 95% floor)"
if ! bash tools/coverage_check.sh >"$log" 2>&1; then
    tail -20 "$log" >&2
    die "coverage below 95% — add tests"
fi
ok "coverage >= 95%"
