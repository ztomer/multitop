#!/usr/bin/env bash
# Install multitop to Homebrew's bindir.
#
# Usage: ./install.sh [--backend zigbuild|docker] [--debug] [--prefix DIR]
#
# Builds via ./build.sh, then copies the host binary to
# "$(brew --prefix)/bin" (/opt/homebrew/bin on Apple silicon,
# /usr/local/bin on Intel/Linux). --prefix overrides the destination
# (useful for testing or non-brew layouts).
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
die()  { printf '%sx%s %s\n'  "$RED" "$RST" "$*" >&2; exit 1; }

PREFIX=""
BUILD_ARGS=()
while [ $# -gt 0 ]; do
    case "$1" in
        --prefix) PREFIX="${2:-}"; shift 2 ;;
        --backend) BUILD_ARGS+=("$1" "${2:-}"); shift 2 ;;
        --backend=*) BUILD_ARGS+=("$1"); shift ;;
        --debug) BUILD_ARGS+=("--debug"); shift ;;
        -h|--help) grep '^#' "$0" | cut -c3-; exit 0 ;;
        *) die "Unknown argument '$1'" ;;
    esac
done

if [ -z "$PREFIX" ]; then
    if command -v brew >/dev/null 2>&1; then
        PREFIX="$(brew --prefix)/bin"
    else
        die "brew not found; pass --prefix DIR explicitly"
    fi
fi

info "building multitop"
if [ "${#BUILD_ARGS[@]}" -gt 0 ]; then
    ./build.sh "${BUILD_ARGS[@]}"
else
    ./build.sh
fi

# Resolve the binary the same way build.sh does: cargo's real target dir,
# not a hardcoded "target" (a shared CARGO_TARGET_DIR relocates artifacts).
PROFILE="release"
if [ "${#BUILD_ARGS[@]}" -gt 0 ]; then
    for a in "${BUILD_ARGS[@]}"; do
        [ "$a" = "--debug" ] && PROFILE="debug"
    done
fi
[ -d "$HOME/.cargo/bin" ] && PATH="$HOME/.cargo/bin:$PATH"
export PATH
TARGET_DIR="$(cargo metadata --format-version 1 2>/dev/null | jq -r .target_directory)"
[ -n "$TARGET_DIR" ] && [ "$TARGET_DIR" != null ] || TARGET_DIR="$PWD/target"
BIN="$TARGET_DIR/$PROFILE/multitop"
[ -f "$BIN" ] || die "binary missing at $BIN"

[ -d "$PREFIX" ] || die "destination $PREFIX does not exist"
info "installing $BIN ${BOLD}->${RST} $PREFIX/multitop"
cp -f "$BIN" "$PREFIX/multitop"
chmod 755 "$PREFIX/multitop"

# Keep the ad-hoc signature stable so keychain "Always Allow" survives
# reinstalls (same identifier build.sh signs with).
if command -v codesign >/dev/null 2>&1; then
    codesign -s - --identifier com.ztomer.multitop "$PREFIX/multitop" 2>/dev/null || true
fi

ok "installed $PREFIX/multitop"
printf '\n%sRun it:%s  multitop\n' "$BOLD" "$RST"
