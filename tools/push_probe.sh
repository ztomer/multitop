#!/usr/bin/env bash
# Diagnose the push path in seconds, stage by stage.
#
# `git push --dry-run` is the trap this exists for: ref discovery on a
# public repo is anonymous, so a dry-run "success" proves nothing about
# authentication -- the POST that carries the pack is a different request
# with different credentials, and that is exactly the one that stalls.
# Each stage below names what it proves; the first red line is the answer.
#
# Usage: tools/push_probe.sh
# Exit 0 when every stage passes, 1 naming the failing one.
set -euo pipefail

root="$(git rev-parse --show-toplevel)"
cd "$root"

pass() { printf '  ok %s\n' "$1"; }
fail() {
	printf '  FAIL %s\n' "$1" >&2
	exit 1
}

origin_url="$(git remote get-url origin 2>/dev/null)" || fail "no origin remote"
slug="$(printf '%s' "$origin_url" | sed -E 's#.*github\.com[:/]([^/]+/[^/]+)(\.git)?$#\1#;s#\.git$##')"

# 1. Reachability: anonymous ref discovery. Proves network + remote exists.
if timeout 30 git ls-remote origin HEAD >/dev/null 2>&1; then
	pass "reachability (anonymous ls-remote answers)"
else
	fail "reachability (anonymous ls-remote failed -- network or remote?)"
fi

# 2. Authentication: the same endpoint git will POST to, with credentials.
# curl, not git, so a stall here cannot hide inside git's retry logic.
token="$(gh auth token 2>/dev/null)" || token=""
if [ -z "$token" ]; then
	fail "authentication (no token: run gh auth login)"
fi
code="$(curl -s -o /dev/null -w "%{http_code}" --max-time 25 -u "git:$token" \
	"https://github.com/${slug}.git/info/refs?service=git-receive-pack")"
[ "$code" = "200" ] || fail "authentication (authed discovery returned $code, not 200)"
pass "authentication (authed discovery returned 200)"

# 3. Authorization: does this identity have push permission at all?
perm="$(gh api "repos/$slug" --jq .permissions.push 2>/dev/null)" || perm=""
[ "$perm" = "true" ] || fail "authorization (permissions.push is not true)"
pass "authentication + authorization (token valid, can push)"

# 4. Informational: what a push would actually send. A huge pack explains a
# slow POST; a tiny one that still stalls points at the transport, not the
# payload. Neither fails the probe.
count="$(git rev-list --count "origin/main..main" 2>/dev/null || echo ?)"

pass "probe complete ($count commits ahead of origin/main)"
