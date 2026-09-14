#!/usr/bin/env bash
# The fuzz targets still compile against what they fuzz. Two layers:
#   1. `cargo check` over fuzz/Cargo.toml -- always, cheap, stable toolchain.
#      This is what catches the failure that actually happens: the protocol
#      gained a variant and nobody rebuilt the protocol fuzzer.
#   2. `cargo +nightly fuzz build <target>` per target -- the full ASan build,
#      when cargo-fuzz and a nightly toolchain are present; a named skip
#      otherwise. Named ON PURPOSE: nightly stays out of `rustup default`
#      (it ICEs on multitop-vault under clippy), and `-Zsanitizer=address` is
#      nightly-only. Do not "simplify" to `cargo fuzz -s none` on stable: it
#      builds, and it turns the vault fuzzers into panic-catchers -- the
#      memory errors ASan exists for go undetected while the gate looks green.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"
GOH="${GOH_DIR:-${GOH:-$HOME/Projects/gates_of_heck}}"
# shellcheck source=/Users/ztomer/Projects/gates_of_heck/tui/lib.sh
. "$GOH/tui/lib.sh"
targets=(fuzz/fuzz_targets/*.rs)
[ -e "${targets[0]}" ] || die "no fuzz targets found -- fuzz/fuzz_targets is empty"
info "fuzz targets compile (cargo check)"
cargo check --manifest-path fuzz/Cargo.toml --all-targets --quiet
ok "fuzz targets compile"
if ! cargo +nightly fuzz --version >/dev/null 2>&1; then
    warn "cargo-fuzz on nightly is not installed -- the ASan build is SKIPPED (cargo install cargo-fuzz; rustup toolchain install nightly)"
    exit 0
fi
for t in "${targets[@]}"; do
    name="$(basename "$t" .rs)"
    info "cargo +nightly fuzz build $name"
    cargo +nightly fuzz build "$name" >/dev/null 2>&1 || die "fuzz target $name does not build under ASan"
done
ok "fuzz targets build under AddressSanitizer"
