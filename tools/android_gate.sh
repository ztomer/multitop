#!/usr/bin/env bash
# The Android companion's gate (android/, docs/roadmap.md A2): ktlint, Android
# lint and the Kotlin compiler with warnings as errors, the JVM unit tests, and
# a debug APK -- then two checks Gradle cannot make itself:
#
#   * every deprecation the build prints is held, with its reason, in
#     android/gradle_deprecations.txt (tools/android_deprecations.py, a ratchet:
#     a new one fails, and an entry that stopped firing fails until deleted);
#   * the APK packages 64-bit ABIs only.
#
# Run by the pre-push hook (GOH_CI_STEPS) and by CI's android job, the same
# command in both; tools/check_gate_parity.py fails either one dropping it.
#
#   tools/android_gate.sh               the gate
#   tools/android_gate.sh --check-host  only the build-host policy (tests/test_android_gate.py)
#
# Build hosts (house platform policy): macOS on Apple silicon, Linux on x86_64
# or aarch64. Anything else is refused, never built "best effort".
# MONITOR_ANDROID_BUILD_OS / MONITOR_ANDROID_BUILD_ARCH override uname, so both
# directions of that rule are testable on one machine.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"
GOH="${GOH_DIR:-${GOH:-$HOME/Projects/gates_of_heck}}"
# shellcheck source=/Users/ztomer/Projects/gates_of_heck/tui/lib.sh
. "$GOH/tui/lib.sh"

os="${MONITOR_ANDROID_BUILD_OS:-$(uname -s)}"
arch="${MONITOR_ANDROID_BUILD_ARCH:-$(uname -m)}"
case "$os/$arch" in
    Darwin/arm64 | Linux/x86_64 | Linux/aarch64) ;;
    *) die "unsupported build host $os/$arch -- the Android gate runs on macOS/arm64, Linux/x86_64 or Linux/aarch64 only" ;;
esac
if [ "${1:-}" = --check-host ]; then
    ok "build host $os/$arch"
    exit 0
fi

# The SDK. Never skipped: a missing SDK is a red gate that says how to fix it.
sdk="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-}}"
if [ -z "$sdk" ] && [ "$os" = Darwin ] && [ -d "$HOME/Library/Android/sdk" ]; then
    sdk="$HOME/Library/Android/sdk"
fi
if [ -z "$sdk" ] || [ ! -d "$sdk" ]; then
    err "no Android SDK: ANDROID_HOME is unset or not a directory${sdk:+ ($sdk)}"
    info "install it: brew install --cask android-commandlinetools (macOS), or Google's cmdline-tools zip (Linux)"
    info "then: sdkmanager --licenses && sdkmanager 'platform-tools' 'platforms;android-37.2'"
    die "export ANDROID_HOME=<sdk dir> and re-run tools/android_gate.sh"
fi
[ -f "$sdk/licenses/android-sdk-license" ] ||
    die "the SDK licences at $sdk are not accepted -- run: sdkmanager --licenses"
command -v java >/dev/null 2>&1 || die "no JDK on PATH -- install one (macOS: brew install --cask temurin)"
export ANDROID_HOME="$sdk"

log="$(mktemp "${TMPDIR:-/tmp}/multitop-android.XXXXXX")"
trap 'rm -f "$log"' EXIT

section "android companion"
# `clean` first: a task Gradle skips as up to date prints none of its warnings,
# so the deprecation ratchet below would see a different log on every run.
info "ktlint, lint, unit tests and a debug build from clean (warnings are errors)"
if ! (cd android && ./gradlew --no-daemon --console=plain \
    clean ktlintCheck lintDebug testDebugUnitTest assembleDebug) >"$log" 2>&1 </dev/null; then
    python3 "$GOH/lib/fail_lines.py" "$log" >&2 || true
    tail -60 "$log" >&2
    die "the Android build, lint or unit tests failed"
fi
ok "ktlint, lint, unit tests and build"

python3 tools/android_deprecations.py --self-test >/dev/null ||
    die "android_deprecations.py's self-test failed -- it cannot be trusted"
python3 tools/android_deprecations.py "$log" android/gradle_deprecations.txt ||
    die "the build's deprecations do not match android/gradle_deprecations.txt"
ok "deprecations held by the ratchet, none new"

apk=android/app/build/outputs/apk/debug/app-debug.apk
[ -f "$apk" ] || die "Gradle reported success but wrote no $apk"
bad="$(python3 -c '
import sys, zipfile
abis = {n.split("/")[1] for n in zipfile.ZipFile(sys.argv[1]).namelist() if n.startswith("lib/") and n.count("/") >= 2}
print(" ".join(sorted(abis - {"arm64-v8a", "x86_64"})))
' "$apk")"
[ -z "$bad" ] || die "the APK packages non-64-bit ABIs: $bad (abiFilters in android/app/build.gradle.kts)"
ok "64-bit ABIs only"
