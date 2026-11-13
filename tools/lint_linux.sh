#!/usr/bin/env bash
# Lint the WHOLE workspace on Linux (glibc, the CI configuration) in a
# container, with the same system libraries CI installs. The musl agent lint
# in .gatesrc covers the crate that ships to Linux; vault and multitop need
# dbus/tss headers to build there at all, so their Linux half -- every
# #[expect] that is fulfilled on one OS only -- was invisible until CI. This
# is that CI job, runnable before the push (Colima or Docker Desktop).
#
#   tools/lint_linux.sh          # clippy --workspace --all-targets --all-features, -D warnings
#
# A PASS IS PROVEN, NEVER INFERRED (MON.LINTLINUX, 2026-11-13). The step once read PASSED while
# its container died of input/output errors on a full disk: the verdict was the container's exit
# status, in a script whose `a && b` lines exempt `a` from errexit, whose tar pipe had no
# pipefail, and whose every line of output went to /dev/null -- so a lost verdict looked like a
# clean one. Now the container prints MONITOR_LINT_LINUX_OK <rustc --version> as its LAST act,
# after clippy succeeded on the pinned compiler, and this script passes only on exit 0 AND that
# line. Anything else is FAILED, with the log's tail.
#
# The toolchain is ONE pin (RUST_VERSION): it names the base image, the built image, and
# RUSTUP_TOOLCHAIN, which overrides rust-toolchain.toml's floating `stable` so the container
# cannot fetch a different compiler at run time (rust:1.99.1 pulled 1.100 on every run). Raise it
# with the host's stable (`rustc --version`), in its own commit.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
GOH="${GOH_DIR:-$HOME/Projects/gates_of_heck}"
# shellcheck source=/dev/null # goh's tui/lib.sh: outside this repo; gates_of_heck lints it
. "$GOH/tui/lib.sh"

RUST_VERSION="${MONITOR_LINT_RUST_VERSION:-1.100.0}"
DEBIAN=bookworm
BASE_IMAGE="rust:$RUST_VERSION-$DEBIAN"
IMAGE="monitor-linux-lint:$RUST_VERSION-$DEBIAN"
# Named volumes, not host paths and not the container's writable layer: the cargo target and
# registry live on the VM's own Linux filesystem (the Linux artifacts must not land in the repo's
# target/, and a bind mount through virtiofs lost exec bits for divoom-control, DV.LINTEXEC),
# and they persist across `colima stop`, so the cache stays warm.
TARGET_VOLUME="${MONITOR_LINT_TARGET_VOLUME:-monitor-linux-lint-target}"
CARGO_VOLUME="${MONITOR_LINT_CARGO_VOLUME:-monitor-linux-lint-cargo}"
SENTINEL="MONITOR_LINT_LINUX_OK"
LOG="${TMPDIR:-/tmp}/monitor_lint_linux.log"

# Start the container runtime when it is down, and stop it after only if goh
# started it and no other gate still holds it (goh lib/container_runtime.sh).
# shellcheck source=/dev/null # goh's lib/container_runtime.sh: outside this repo; gates_of_heck lints it
. "$GOH/lib/container_runtime.sh"
trap goh_container_runtime_down EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
# The container runs the HOST's architecture: arm64 on the owner's Apple-silicon Mac, amd64 on an
# x86_64 Linux runner, which cannot exec an arm64 image (multitop run 42926394892: "exec format
# error"). `ARCH` is the seam (house rule 3); a 32-bit or unknown host is refused, never guessed.
case "${ARCH:-$(uname -m)}" in
    arm64|aarch64) PLATFORM=linux/arm64 ;;
    x86_64|amd64) PLATFORM=linux/amd64 ;;
    *) err "lint_linux: unsupported host architecture '${ARCH:-$(uname -m)}' (64-bit arm64 or x86_64 only)"; exit 2 ;;
esac
goh_container_runtime_up "monitor lint_linux" || { err "lint_linux: FAILED -- no container runtime"; exit 1; }

# fail_with <what>: the one way out on a bad verdict. Shows the log's tail (the container's own
# words), names FAILED, and exits non-zero. `exit`, not `return`: nothing here may fall through.
fail_with() {
    [ -s "$LOG" ] && tail -n 40 "$LOG" >&2
    err "lint_linux: FAILED -- $1 (full log: $LOG)"
    exit 1
}

# The image: the named base plus the system libraries CI installs and the clippy component, built
# ONCE under a tag that carries the toolchain (raising the pin builds a new image). Baked in, so
# a run needs no apt and no rustup network step to start.
: > "$LOG"
if ! "$DOCKER" image inspect "$IMAGE" >/dev/null 2>&1; then
    info "building $IMAGE (one time: $BASE_IMAGE + libdbus-1-dev libtss2-dev pkg-config + clippy)"
    printf 'FROM %s\nRUN set -eux; apt-get update -qq; apt-get install -y -qq libdbus-1-dev libtss2-dev pkg-config; rm -rf /var/lib/apt/lists/*; rustup component add clippy\n' \
        "$BASE_IMAGE" | "$DOCKER" build -t "$IMAGE" - >"$LOG" 2>&1 \
        || fail_with "image build"
fi

info "clippy on $PLATFORM ($IMAGE, rustc $RUST_VERSION)"
# The host's ~/.cargo/config.toml (sccache wrapper, shared target dir) must
# not leak in: the source is copied and its .cargo/config.toml dropped.
set +e
"$DOCKER" run --rm --platform "$PLATFORM" -v "$ROOT":/src:ro \
  -v "$TARGET_VOLUME":/cache/target -v "$CARGO_VOLUME":/cache/cargo \
  -e CARGO_TARGET_DIR=/cache/target -e CARGO_HOME=/cache/cargo -e RUSTC_WRAPPER= \
  -e RUSTUP_TOOLCHAIN="$RUST_VERSION" -e WANT_RUST="$RUST_VERSION" -e SENTINEL="$SENTINEL" \
  -e PATH="/usr/local/cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin" \
  -w /tmp "$IMAGE" bash -c '
    # pipefail and nounset are what the old `a && b` lines and the bare tar pipe lacked: every
    # failure here stops the script, and the SENTINEL below is reachable only past all of them.
    set -euo pipefail
    # Copy the tree without .git, target, or the host .cargo -- and say so when
    # the copy fails.
    #
    # It used to be `cp -r /src /tmp/src && cd /tmp/src && rm -rf target
    # .cargo/config.toml`, and on any machine whose repository has a git
    # fsmonitor socket in it, `cp` cannot stat that socket through the bind
    # mount and fails. Because the three commands shared one `&&` line, the
    # failure skipped only the `cd`, and cargo then ran in the image WORKDIR
    # (`/tmp`) and reported `could not find Cargo.toml in /tmp` -- a message
    # naming neither the cause nor the directory it was in.
    #
    # tar with excludes, because clippy needs none of the three: no history,
    # no host build cache, and -- the reason the host config must not leak --
    # no sccache wrapper and no shared target dir, either of which would point
    # the container at paths that do not exist inside it.
    mkdir -p /tmp/src
    tar -C /src --exclude=./.git --exclude=./target --exclude=./.cargo -cf - . \
      | tar -C /tmp/src -xf -
    cd /tmp/src
    test -f Cargo.toml || { echo "the copy produced no Cargo.toml" >&2; exit 1; }
    ver="$(rustc --version)"
    case "$ver" in
      "rustc $WANT_RUST "*) ;;
      *) echo "toolchain drift: the image runs \"$ver\", the pin is $WANT_RUST" >&2; exit 1 ;;
    esac
    cargo clippy --workspace --all-targets --all-features -- -D warnings
    echo "$SENTINEL $ver"' >"$LOG" 2>&1
rc=$?
set -e
[ "$rc" -eq 0 ] || fail_with "the container exited $rc"
# Exit 0 is not proof: the verdict is the sentinel, on the pinned compiler.
got="$(grep -m 1 "^$SENTINEL " "$LOG" || true)"
[ -n "$got" ] || fail_with "the container exited 0 but never reported clippy clean (no $SENTINEL line)"
case "$got" in
    "$SENTINEL rustc $RUST_VERSION "*) ;;
    *) fail_with "clippy ran on '${got#"$SENTINEL "}', not the pinned rustc $RUST_VERSION" ;;
esac
ok "linux clippy clean (${got#"$SENTINEL "})"
