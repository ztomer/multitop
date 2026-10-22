#!/usr/bin/env bash
# Install (or remove) the launchd agent that runs the Ops poll.
#
#   tools/install_ops_poller.sh [--uninstall]
#
# WHAT THIS AGENT IS: the Ops poll. It gathers the hosts whose answer is
# due and keeps what it found. That is all it does.
#
# WHY IT EXISTS (servers ROADMAP E36): the Ops poll was bound to the Ops
# VIEW - it ran while a human was looking at the panel and never otherwise
# - so the estate's fingerprint tools fired 0.3 a day while the shared
# tools fired 87. The Ops view was shipped, installed, and nothing asked the
# estate for it. The poll is a JOB, and a job that runs only when somebody
# is watching is not a job.
#
# WHY IT IS A PASS AND NOT A DAEMON: a pass that exits cannot be left
# half-dead. The next pass opens a fresh session, and a session that breaks
# is dropped rather than kept. The answer each pass found is kept beside
# the config, so the Ops view shows what the pass found instead of asking
# the estate a second time.
#
# THE TICK IS NOT CARRIED HERE (servers ROADMAP E53). launchd fixes an
# interval when the job is loaded, so a number in this file was a rate the
# estate's publication could move under: had the estate moved from every 15
# minutes to every 5, this agent would still have read every 6.5 minutes -
# slower than the publication, so its verdict could be two publications
# behind, and the client's own warning would have fired on every tick with
# nothing anyone could do about it but reinstall.
#
# So the agent paces ITSELF: it reads the cadence the estate publishes with
# its verdict and waits the rate that cadence implies (DECISIONS D15). The
# job this template installs therefore carries NO rate at all - it keeps the
# agent ALIVE, and that is all it does. There is no way to hand this script
# a rate either: `--interval` is refused, because a rate in this repo is
# the defect the tick was raised about.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
GOH="${GOH_DIR:-$HOME/Projects/gates_of_heck}"
if [ -f "$GOH/tui/lib.sh" ]; then
    # shellcheck source=/dev/null
    source "$GOH/tui/lib.sh"
fi
info() { printf -- '-> %s\n' "$*"; }
ok() { printf -- '+ %s\n' "$*"; }
die() { printf -- 'x %s\n' "$*" >&2; exit 1; }

label="com.ztomer.multitop.ops-poll"
plist="$HOME/Library/LaunchAgents/$label.plist"
support="$HOME/Library/Application Support/multitop"
runner="$support/ops-poll-agent.sh"
log_dir="$HOME/Library/Logs"
log="$log_dir/multitop-ops-poll.log"
# The minimum gap between two restarts of a FAILED agent.
#
# This is a LIVELINESS bound and not a rate: it says how long launchd waits
# before it tries the agent again, and nothing about how often the agent
# reads the estate. The tick - the rate the agent keeps - is the cadence
# the estate publishes, read by the agent itself (servers ROADMAP E53).
throttle=300
domain="gui/$(id -u)"

uninstall() {
    launchctl bootout "$domain/$label" 2>/dev/null || true
    rm -f "$plist" "$runner"
    ok "removed $label"
    exit 0
}

while [ $# -gt 0 ]; do
    case "$1" in
        --uninstall) uninstall ;;
        --interval)
            die "--interval is gone: the tick is the cadence the estate publishes, read by the agent itself (servers ROADMAP E53). This script installs no rate."
            ;;
        *) die "usage: $0 [--uninstall]" ;;
    esac
done

# The INSTALLED binary, not the build tree's: the agent is a service of the
# shipped client, and a `cargo clean` must not be able to silence it.
bin="$(command -v multitop || true)"
[ -n "$bin" ] || die "multitop is not on PATH - run ./install.sh first"

mkdir -p "$HOME/Library/LaunchAgents" "$log_dir" "$support"

# launchd runs a small wrapper, not the binary directly.
#
# Pointing launchd straight at a binary meant a rebuild silently disabled
# the agent: launchd cannot spawn a missing program, so the one thing built
# to make the poll visible would itself go quiet. The wrapper is installed
# outside the build tree and therefore always spawnable, and when the binary
# is missing it says so in the log rather than going silent.
info "writing the wrapper to $runner"
cat > "$runner" <<EOF
#!/usr/bin/env bash
# Installed by tools/install_ops_poller.sh. Edit that, not this.
set -uo pipefail
bin="$bin"

# launchd launches this wrapper with a bare PATH (/usr/bin:/bin:/usr/sbin:/sbin).
# ssh has to be findable, or the poll cannot reach a single host.
export PATH="\$HOME/.cargo/bin:\$HOME/.local/bin:/opt/homebrew/bin:\$HOME/bin:\$PATH"

if [ ! -x "\$bin" ]; then
    echo "the Ops poll is disabled: \$bin is missing. Run ./install.sh." >&2
    exit 4
fi

# The AGENT, not one pass: it paces itself by the cadence the estate
# publishes with its verdict (servers ROADMAP E53). "exec" keeps the agent
# the job's own process, so launchd's signals reach it and its exit code is
# the job's exit code.
exec "\$bin" --ops-agent
EOF
chmod +x "$runner"

# The job this one replaces may have carried an interval - a rate the estate's
# cadence could move under. Say what is being dropped, so an operator who
# remembers that number knows where it went.
if [ -f "$plist" ]; then
    old="$(plutil -extract StartInterval raw "$plist" 2>/dev/null || true)"
    if [ -n "$old" ]; then
        info "the job being replaced carried an interval of ${old}s - dropped: the agent paces itself by the published cadence"
    fi
fi

cat > "$plist" <<EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>$label</string>
    <key>ProgramArguments</key>
    <array>
        <string>$runner</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <dict>
        <key>SuccessfulExit</key>
        <false/>
    </dict>
    <key>ThrottleInterval</key>
    <integer>$throttle</integer>
    <key>StandardOutPath</key>
    <string>$log</string>
    <key>StandardErrorPath</key>
    <string>$log</string>
    <key>ProcessType</key>
    <string>Background</string>
</dict>
</plist>
EOF

# `bootout`/`bootstrap`, not `load`/`unload`: the latter pair has been
# deprecated for a decade, and the standing rule is to migrate on sight
# rather than wait for it to become an error.
launchctl bootout "$domain/$label" 2>/dev/null || true
launchctl bootstrap "$domain" "$plist"

ok "$label installed - it keeps the agent alive, and the agent paces itself"
info "log: $log"
info "prove one pass by hand:  $bin --ops-poll"
info "prove the agent by hand:  $bin --ops-agent"
