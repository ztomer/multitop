#!/usr/bin/env bash
# Build multitop: cross-compile the Linux agents, then embed them in the
# local binary.
#
# The agent is a static musl binary uploaded to each monitored host, so it
# must be built for Linux even when you develop on macOS. Two backends are
# supported; the first available one is used.
#
#   zigbuild  cargo-zigbuild + rustup musl targets  (fast, no daemon)
#   docker    rust:<version>-alpine container       (no local toolchain setup)
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

# >>> docker backend
# A named release, never rust:alpine (house rule: latest is a named release). The pin names the
# image AND RUSTUP_TOOLCHAIN, which overrides rust-toolchain.toml's floating `stable` so the
# container cannot fetch another compiler. Raise it with the host's stable, in its own commit.
AGENT_RUST_VERSION="${MULTITOP_AGENT_RUST_VERSION:-1.100.0}"
AGENT_ALPINE="${MULTITOP_AGENT_ALPINE:-alpine3.23}"
AGENT_IMAGE="rust:$AGENT_RUST_VERSION-$AGENT_ALPINE"
# The cargo target and registry live in docker NAMED volumes, on the VM's own Linux filesystem,
# never on a bind mount of the checkout: a build script on a colima virtiofs mount lost its exec
# bit intermittently ("Permission denied" running build-script-build, DV.LINTEXEC, EST.HOSTMOUNTTARGET).
# The source is mounted read-only; the ONE host mount that is written is the output directory,
# which receives the finished agent as a plain file where release.sh and
# tools/check_agent_version.py read it (target/docker/<triple>/<profile>/multitop-agent).
AGENT_TARGET_VOLUME="${MULTITOP_AGENT_TARGET_VOLUME:-multitop-agent-target}"
AGENT_CARGO_VOLUME="${MULTITOP_AGENT_CARGO_VOLUME:-multitop-agent-cargo}"

build_agent_docker() {
    local target="$1" out="$PWD/target/docker"
    mkdir -p "$out"
    # RUSTC_WRAPPER= : the checkout's .cargo/config.toml names sccache, which the image lacks.
    docker run --rm \
        -v "$PWD":/src:ro -w /src \
        -v "$AGENT_TARGET_VOLUME":/cache/target -v "$AGENT_CARGO_VOLUME":/cache/cargo \
        -v "$out":/out \
        -e CARGO_TARGET_DIR=/cache/target -e CARGO_HOME=/cache/cargo -e RUSTC_WRAPPER= \
        -e RUSTUP_TOOLCHAIN="$AGENT_RUST_VERSION" \
        "$AGENT_IMAGE" sh -c "
            set -eu
            apk add --no-cache musl-dev >/dev/null
            rustup target add $target >/dev/null
            cargo build -p multitop-agent --target $target $CARGO_PROFILE_FLAG
            mkdir -p /out/$target/$PROFILE
            cp /cache/target/$target/$PROFILE/multitop-agent /out/$target/$PROFILE/multitop-agent
        "
}
# <<< docker backend

agent_path() {
    local target="$1" root="$TARGET_DIR"
    # The docker backend builds in a named volume and copies the agent out to
    # the project-local target/docker, regardless of the host's target dir.
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

# Stabilize ad-hoc signature so keychain "Always Allow" persists across builds.
# Without a fixed identifier each build gets a random ad-hoc ID and the
# keychain sees it as a different app.
if command -v codesign >/dev/null 2>&1; then
    for _p in "$TARGET_DIR/$PROFILE/multitop" "$HOME/.cargo/bin/multitop" "$TARGET_DIR/debug/multitop" "$TARGET_DIR/release/multitop"; do
        [ -f "$_p" ] && codesign -s - --identifier com.ztomer.multitop "$_p" 2>/dev/null || true
    done
fi

BIN="$TARGET_DIR/$PROFILE/multitop"
ok "built $BIN ($(($(wc -c < "$BIN" | tr -d ' ') / 1024)) KiB)"
# Print a path that actually runs from the repo: relative when the binary is
# inside it, bare absolute otherwise ("./$abs" would point at ./Users/...).
if [ "$BIN" = "${BIN#"$PWD"/}" ]; then RUN_BIN="$BIN"; else RUN_BIN="./${BIN#"$PWD"/}"; fi
printf '\n%sRun it:%s  %s\n' "$BOLD" "$RST" "$RUN_BIN"
