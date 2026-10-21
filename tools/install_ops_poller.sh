#!/usr/bin/env bash
# Install (or remove) the launchd agent that runs the Ops poll.
#
#   tools/install_ops_poller.sh [--interval SECONDS] [--uninstall]
#
# WHAT THIS AGENT IS: the Ops poll. Every interval it runs `multitop
# --ops-poll`, which gathers the hosts whose answer is due and keeps it.
# That is all it does.
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
# THE INTERVAL IS THE RULE (DECISIONS D15): the estate publishes its
# verdict every 15 minutes, so a client reads at least every 7.5 minutes.
# The default here is that number; the pass decides for itself what is due.
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
# The scheduler's tick, a little under the poll's interval: launchd's timer
# wheel added about 30 s to a 450 s `StartInterval` on this Mac (measured
# 2026-10-21), which would put the client at 8 minutes a gather - the rate
# the routines client has, and one step outside the rule's 7.5. At 420 s
# the gathers land every 7 to 7.5 minutes, inside it.
interval=420
domain="gui/$(id -u)"

uninstall() {
    launchctl bootout "$domain/$label" 2>/dev/null || true
    rm -f "$plist" "$runner"
    ok "removed $label"
    exit 0
}

while [ $# -gt 0 ]; do
    case "$1" in
        --interval) interval="$2"; shift 2 ;;
        --uninstall) uninstall ;;
        *) die "usage: $0 [--interval SECONDS] [--uninstall]" ;;
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
exec "\$bin" --ops-poll
EOF
chmod +x "$runner"

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
    <key>StartInterval</key>
    <integer>$interval</integer>
    <key>RunAtLoad</key>
    <true/>
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

ok "$label installed - looks every ${interval}s"
info "log: $log"
info "prove it by hand:  $bin --ops-poll"
