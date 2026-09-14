#!/usr/bin/env bash
# Lint the WHOLE workspace on Linux (glibc, the CI configuration) in a
# container, with the same system libraries CI installs. The musl agent lint
# in .gatesrc covers the crate that ships to Linux; vault and multitop need
# dbus/tss headers to build there at all, so their Linux half -- every
# #[expect] that is fulfilled on one OS only -- was invisible until CI. This
# is that CI job, runnable before the push (Colima or Docker Desktop).
#
#   tools/lint_linux.sh          # clippy --workspace --all-targets --all-features, -D warnings
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
GOH="${GOH_DIR:-$HOME/Projects/gates_of_heck}"
# shellcheck source=/Users/ztomer/Projects/gates_of_heck/tui/lib.sh
. "$GOH/tui/lib.sh"
docker info >/dev/null 2>&1 || die "no container runtime answering (colima start)"
info "clippy on linux/arm64 (rust:1-bookworm + libdbus-1-dev libtss2-dev)"
# The host's ~/.cargo/config.toml (sccache wrapper, shared target dir) must
# not leak in: the source is copied and its .cargo/config.toml dropped.
docker run --rm --platform linux/arm64 -v "$ROOT":/src:ro \
  -e CARGO_TARGET_DIR=/tmp/target -e RUSTC_WRAPPER= -w /tmp rust:1-bookworm bash -c '
    set -e
    apt-get update -qq >/dev/null && apt-get install -y -qq libdbus-1-dev libtss2-dev pkg-config >/dev/null 2>&1
    rustup component add clippy >/dev/null 2>&1
    cp -r /src /tmp/src && cd /tmp/src && rm -rf target .cargo/config.toml
    cargo clippy --workspace --all-targets --all-features -- -D warnings'
ok "linux clippy clean"
