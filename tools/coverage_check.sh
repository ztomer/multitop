#!/usr/bin/env bash
# Coverage gate for the workspace — delegating shim.
#
# The gate itself lives in gates_of_heck (gates/coverage_gate.sh --lang rust),
# which every caller here already reaches through this one path: the pre-commit
# hook, tools/repo_gates.sh, CI and scripts/local-ci.py. One definition of the
# floor and its exclusions; none of them can drift weaker than another.
#
# 95% floor on LINE coverage, ignoring files that are inherently untestable in
# CI: they take over the real terminal, talk to the OS credential store or the
# Secure Enclave, spawn real SSH children, or run at build time rather than
# under the suite. Those exclusions are structural — each one is named in the
# regex below with its reason so the list cannot quietly grow.
#
# The rule for adding one: the file must be untestable *by construction*, not
# merely untested. If a seam would make it testable, add the seam instead.
#
# Ignored and why:
#   ssh.rs            — real SSH transport, needs a live host (#[ignore]d tests)
#   password_store.rs — OS keychain; the mock covers the logic, the OS path is
#                       CI-only and a real read puts a dialog on screen
#   spawn.rs          — spawns real SSH/agent child processes
#   main.rs           — entry point, exercised by the binary not the suite
#   entry.rs          — (multitop `run/`) the same, one level down: `run()`
#                       calls `ratatui::init` and reads the process's own stdin,
#                       so a test running it fights the developer for the
#                       terminal it is printing into. Everything decidable
#                       without a terminal lives in `run/`'s other modules and
#                       is tested there.
#   build.rs          — build script; cargo runs it while compiling, never from
#                       a test binary
#   sparkline.rs      — pure rendering, no branching logic to test
#   secure_enclave.rs — (vault crate) macOS Security framework
#   sys.rs            — (agent crate) /proc parsing, Linux-only
#   fprintd.rs        — (vault crate) Linux fingerprint daemon
#   tpm2.rs           — (vault crate) sealing the vault key to a TPM. Needs TPM
#                       hardware and `tss` group access to /dev/tpmrm0, so CI
#                       can execute none of it. The round trip is an
#                       `#[ignore]`d test run by hand on a machine that has one
#                       -- visibly not run, rather than a gate reporting success
#                       for doing nothing. The framing around the sealed blob is
#                       pure and IS tested.
#   enclave.rs        — (vault `api/`) unlocking by touch and repairing the
#                       enclave wrapper. Needs Secure Enclave hardware holding a
#                       key bound to this machine's enrolled biometric set, so
#                       no test can execute it. Split out of `unlock.rs` for
#                       this reason: mixed in, it hid which of the *password*
#                       paths were genuinely untested. The decision it feeds —
#                       `Vault::biometric_available`, which picks the door the
#                       user is shown — stays in `unlock.rs` and is tested.

set -euo pipefail

cd "$(git rev-parse --show-toplevel 2>/dev/null || pwd)"

GOH="${GOH_DIR:-$HOME/Projects/gates_of_heck}"

"$GOH/gates/coverage_gate.sh" --lang rust --floor 95 \
    --ignore 'ssh\.rs|password_store\.rs|sparkline\.rs|main\.rs|entry\.rs|build\.rs|spawn\.rs|sys\.rs|secure_enclave\.rs|fprintd\.rs|enclave\.rs|tpm2\.rs'

# CI uploads target/lcov.info as a workflow artifact (if-no-files-found:
# error). The central gate writes per-target exports under
# target/llvm-cov/lcov-parts/; concatenate them so that contract keeps
# holding without re-running anything.
parts=(target/llvm-cov/lcov-parts/part-*.info)
if [ -e "${parts[0]}" ]; then
    : >target/lcov.info
    for p in "${parts[@]}"; do
        cat "$p" >>target/lcov.info
    done
fi
