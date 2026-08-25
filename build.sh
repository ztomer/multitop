#!/usr/bin/env bash
# Build multitop: cross-compile the Linux agents, then embed them in the
# local binary.
#
# The agent is a static musl binary uploaded to each monitored host, so it
# must be built for Linux even when you develop on macOS. Two backends are
# supported; the first available one is used.
#
#   zigbuild  cargo-zigbuild + rustup musl targets  (fast, no daemon)
#   docker    rust:alpine container                 (no local toolchain setup)
#
# Usage: ./build.sh [--backend zigbuild|docker] [--debug]
set -euo pipefail

cd "$(dirname "$0")"

if [ -t 1 ] && [ -z "${NO_COLOR:-}" ]; then
    DIM=$'\033[2m'; BOLD=$'\033[1m'; RED=$'\033[0;31m'; GRN=$'\033[0;32m'
    YEL=$'\033[0;33m'; RST=$'\033[0m'
else
    DIM=''; BOLD=''; RED=''; GRN=''; YEL=''; RST=''
fi
info() { printf '%s->%s %s\n' "$DIM" "$RST" "$*"; }
ok()   { printf '%s+%s %s\n'  "$GRN" "$RST" "$*"; }
warn() { printf '%s!%s %s\n'  "$YEL" "$RST" "$*"; }
die()  { printf '%sx%s %s\n'  "$RED" "$RST" "$*" >&2; exit 1; }

BACKEND=""
PROFILE="release"
CARGO_PROFILE_FLAG="--release"
while [ $# -gt 0 ]; do
    case "$1" in
        --backend) BACKEND="${2:-}"; shift 2 ;;
        --debug)   PROFILE="debug"; CARGO_PROFILE_FLAG=""; shift ;;
        -h|--help) grep '^#' "$0" | cut -c3-; exit 0 ;;
        *) die "Unknown argument '$1'" ;;
    esac
done

TARGETS="x86_64-unknown-linux-musl aarch64-unknown-linux-musl"

# rustup may be installed without having touched the shell profile, in which
# case its shims are present but not on PATH.
[ -d "$HOME/.cargo/bin" ] && PATH="$HOME/.cargo/bin:$PATH"
export PATH

# Resolve cargo's target directory instead of hardcoding "target": a shared
# CARGO_TARGET_DIR in ~/.cargo/config.toml (machine-wide sccache setup) puts
# artifacts elsewhere, and any hardcoded path silently misses them.
TARGET_DIR="$(cargo metadata --format-version 1 2>/dev/null | jq -r .target_directory)"
[ -n "$TARGET_DIR" ] && [ "$TARGET_DIR" != null ] || TARGET_DIR="$PWD/target"

detect_backend() {
    if command -v cargo-zigbuild >/dev/null 2>&1 && command -v rustup >/dev/null 2>&1; then
        echo zigbuild
    elif docker info >/dev/null 2>&1; then
        echo docker
    else
        echo none
    fi
}

[ -n "$BACKEND" ] || BACKEND="$(detect_backend)"

if [ "$BACKEND" = none ]; then
    cat >&2 <<'EOF'
x No cross-compilation backend found.

  The agent is a static Linux binary; building it from macOS needs one of:

  1. cargo-zigbuild (recommended - no daemon, ~30s setup)

       curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
       rustup target add x86_64-unknown-linux-musl aarch64-unknown-linux-musl
       cargo install cargo-zigbuild      # needs zig, already installed via brew

  2. Docker

       Start Docker Desktop, then re-run ./build.sh

  On a Linux host with the musl targets installed, ./build.sh works as is.
EOF
    exit 1
fi

info "backend: ${BOLD}${BACKEND}${RST}"

build_agent_zigbuild() {
    local target="$1"
    rustup target add "$target" >/dev/null 2>&1 || true
    cargo zigbuild -p multitop-agent --target "$target" $CARGO_PROFILE_FLAG
}

build_agent_docker() {
    local target="$1"
    docker run --rm \
        -v "$PWD":/src -w /src \
        -e CARGO_TARGET_DIR=/src/target/docker \
        rust:alpine sh -c "
            apk add --no-cache musl-dev >/dev/null &&
            rustup target add $target >/dev/null &&
            cargo build -p multitop-agent --target $target $CARGO_PROFILE_FLAG
        "
}

agent_path() {
    local target="$1" root="$TARGET_DIR"
    # The docker backend sets its own CARGO_TARGET_DIR inside the container,
    # so its artifacts land under the project-local target/docker regardless
    # of the host's shared target dir.
    [ "$BACKEND" = docker ] && root="$PWD/target/docker"
    echo "$root/$target/$PROFILE/multitop-agent"
}

for target in $TARGETS; do
    info "building agent for $target"
    "build_agent_$BACKEND" "$target"
    path="$(agent_path "$target")"
    [ -f "$path" ] || die "agent missing at $path"
    size=$(wc -c < "$path" | tr -d ' ')
    ok "$target  $((size / 1024)) KiB"
done

export MULTITOP_AGENT_X86_64="$(agent_path x86_64-unknown-linux-musl)"
export MULTITOP_AGENT_AARCH64="$(agent_path aarch64-unknown-linux-musl)"

info "building multitop for the host"
cargo build -p multitop $CARGO_PROFILE_FLAG

BIN="$TARGET_DIR/$PROFILE/multitop"
ok "built $BIN ($(($(wc -c < "$BIN" | tr -d ' ') / 1024)) KiB)"
# Print a path that actually runs from the repo: relative when the binary is
# inside it, bare absolute otherwise ("./$abs" would point at ./Users/...).
if [ "$BIN" = "${BIN#"$PWD"/}" ]; then RUN_BIN="$BIN"; else RUN_BIN="./${BIN#"$PWD"/}"; fi
printf '\n%sRun it:%s  %s\n' "$BOLD" "$RST" "$RUN_BIN"
