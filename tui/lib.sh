# shellcheck shell=bash
# Kare-style TUI output for this repo's scripts. Source it, don't execute it:
#
#     . "$(git rev-parse --show-toplevel)/tui/lib.sh"
#     section "checks"; info "linting"; ok "clean"; die "clippy failed"
#
# Icons are the permitted Kare set only: → · ✓ ✗ ⚠
# Colors are restrained and disabled automatically when NO_COLOR is set or
# stdout is not a terminal, so piped and CI output stays plain.

if [ -n "${NO_COLOR:-}" ] || [ ! -t 1 ]; then
    _C_DIM=''; _C_RED=''; _C_GRN=''; _C_YEL=''; _C_BLD=''; _C_OFF=''
else
    _C_DIM=$(printf '\033[2m')
    _C_RED=$(printf '\033[31m')
    _C_GRN=$(printf '\033[32m')
    _C_YEL=$(printf '\033[33m')
    _C_BLD=$(printf '\033[1m')
    _C_OFF=$(printf '\033[0m')
fi

# A titled divider introducing a group of steps.
section() { printf '\n%s%s%s\n' "$_C_BLD" "$*" "$_C_OFF"; }

# A horizontal rule.
hr() { printf '%s%s%s\n' "$_C_DIM" "----------------------------------------" "$_C_OFF"; }

# Work starting.
info() { printf '  %s→%s %s\n' "$_C_DIM" "$_C_OFF" "$*"; }

# A neutral detail under the current step.
note() { printf '  %s·%s %s\n' "$_C_DIM" "$_C_OFF" "$*"; }

# Work succeeded.
ok() { printf '  %s✓%s %s\n' "$_C_GRN" "$_C_OFF" "$*"; }

# Something is off but not fatal.
warn() { printf '  %s⚠%s %s\n' "$_C_YEL" "$_C_OFF" "$*" >&2; }

# Work failed.
err() { printf '  %s✗%s %s\n' "$_C_RED" "$_C_OFF" "$*" >&2; }

# Work failed and we stop here.
die() { err "$*"; printf '\n' >&2; exit 1; }
