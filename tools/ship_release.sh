#!/usr/bin/env bash
# ship_release.sh -- the release kit `goh ship` drives (.gatesrc GOH_SHIP_RELEASE, MON.SHIPKIT).
#
#   tools/ship_release.sh 0.69.8
#
# goh ship hands its kit ONE argument, `{version}`, in place of the push step (GOH_SHIP_RELEASE_PUSHES=1).
# ./release.sh takes the version but writes nothing: it refuses unless Cargo.toml already says it
# and CHANGELOG.md already has its stanza, and goh ship decides the number. This is the front half
# it lacks:
#
#   1. Cargo.toml's [workspace.package] version, and the lockfile entries of the crates that follow
#      it (Cargo.lock and fuzz/Cargo.lock), written to VERSION when the tree still says the newest
#      tag's version; a tree that already declares ANOTHER new version is refused, naming
#      `goh ship --version` (a commit decided the number; ship must not override it);
#   2. the `## vVERSION` stanza cut from Unreleased (goh's changelog_cut.py, titled with ship's
#      --summary when there is one), when the file has none yet;
#   3. one `release vVERSION: TITLE` commit of exactly those files, through the hooks;
#   4. ./release.sh vVERSION: the gates, both agents cross-built, ./install.sh (the push gate compares
#      the installed binary with this build, so the install comes BEFORE the push), the annotated tag,
#      ONE atomic push of main and the tag, the GitHub release, the Homebrew tap bump.
#
# No GOH_SHIP_INSTALL is declared beside it: release.sh installs, and an install after the push would
# rebuild what the push gate had already judged. tests/test_ship_release.py pins this file.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
GOH="${GOH_DIR:-$HOME/Projects/gates_of_heck}"
# shellcheck source=/dev/null # goh's tui/lib.sh: outside this repo; gates_of_heck lints it
source "$GOH/tui/lib.sh"
cd "$ROOT"

# The crates that carry `version.workspace = true`: the only lockfile entries a bump moves. The vault
# has a version of its own and registry crates are not ours, so neither is matched by number.
# tests/test_ship_release.py holds this list to the crates' own manifests.
WORKSPACE_CRATES="multitop multitop-agent multitop-testassert"

VERSION="${1:-}"
VERSION="${VERSION#v}"
[[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "usage: tools/ship_release.sh X.Y.Z (got '${1:-}')"
[ -z "$(git status --porcelain --untracked-files=no)" ] || die "the tree has uncommitted changes; a release names a commit"

TAGS="$(git tag --list 'v*' --sort=-v:refname)"
TAG_V="$(grep -E -m 1 '^v[0-9]+\.[0-9]+\.[0-9]+$' <<<"$TAGS" || true)"
TAG_V="${TAG_V#v}"
TREE_V="$(awk '/^\[workspace\.package\]/{p=1; next} /^\[/{p=0} p && /^version = "/{gsub(/"/, "", $3); print $3; exit}' Cargo.toml)"
[ -n "$TREE_V" ] || die "no [workspace.package] version in Cargo.toml"

TITLE=""
if [ -n "${GOH_SHIP_RESULT:-}" ] && [ -f "$GOH_SHIP_RESULT" ]; then
    TITLE="$(bash "$GOH/gates/python.sh" -c 'import json,sys; print((json.load(open(sys.argv[1])).get("summary") or "").strip())' "$GOH_SHIP_RESULT")"
fi

# bump_lock FILE: every WORKSPACE_CRATES entry reading TREE_V is written to VERSION, in place (a
# `cargo update` would resolve the whole graph, where the bump changes one line per crate). Prints how
# many entries it wrote; none is a refusal, because a lockfile left behind makes cargo rewrite it
# mid-release and dirties the tree the tag names.
bump_lock() {
    local file="$1" found
    awk -v old="$TREE_V" -v new="$VERSION" -v names="$WORKSPACE_CRATES" '
        BEGIN { n = split(names, a, " "); for (i = 1; i <= n; i++) ours["name = \"" a[i] "\""] = 1 }
        prev in ours && $0 == "version = \"" old "\"" { print "version = \"" new "\""; prev = ""; next }
        { print; prev = $0 }
    ' "$file" > "$file.ship" && mv "$file.ship" "$file"
    found="$(awk -v new="$VERSION" -v names="$WORKSPACE_CRATES" '
        BEGIN { n = split(names, a, " "); for (i = 1; i <= n; i++) ours["name = \"" a[i] "\""] = 1 }
        prev in ours && $0 == "version = \"" new "\"" { c++ }
        { prev = $0 }
        END { print c + 0 }
    ' "$file")"
    [ "$found" -gt 0 ] || die "$file names none of $WORKSPACE_CRATES at $VERSION after the bump"
}

# A refusal below leaves the checkout as it found it: ship runs in the main checkout.
restore() { git checkout -q -- Cargo.toml Cargo.lock fuzz/Cargo.lock CHANGELOG.md; rm -f Cargo.toml.ship Cargo.lock.ship fuzz/Cargo.lock.ship; }
trap restore EXIT

changed=()
if [ "$TREE_V" != "$VERSION" ]; then
    [ "$TREE_V" = "$TAG_V" ] \
        || die "Cargo.toml already declares $TREE_V (newest tag v$TAG_V), not ship's $VERSION: goh ship --version $TREE_V"
    section "Cargo.toml $TREE_V -> $VERSION"
    awk -v v="$VERSION" '/^\[workspace\.package\]/{p=1; print; next} /^\[/{p=0} p && !done && /^version = "/{print "version = \"" v "\""; done=1; next} {print}' \
        Cargo.toml > Cargo.toml.ship && mv Cargo.toml.ship Cargo.toml
    bump_lock Cargo.lock
    bump_lock fuzz/Cargo.lock
    changed+=(Cargo.toml Cargo.lock fuzz/Cargo.lock)
fi

if ! grep -q "^## v$VERSION" CHANGELOG.md; then
    section "CHANGELOG.md: v$VERSION from Unreleased"
    cut=(cut --version "$VERSION" --file CHANGELOG.md)
    [ -z "$TITLE" ] || cut+=(--title "$TITLE")
    bash "$GOH/gates/python.sh" "$GOH/tools/release-kit/changelog_cut.py" "${cut[@]}" \
        || die "no v$VERSION stanza could be cut (Unreleased empty?); nothing is committed"
    changed+=(CHANGELOG.md)
fi

if [ "${#changed[@]}" -gt 0 ]; then
    HEAD_TITLE="$(sed -n -e "s/^## v$VERSION — \(.*\) _(.*)_\$/\1/p" -e "/^## v$VERSION/q" CHANGELOG.md)"
    git add "${changed[@]}"
    git commit -q -m "release v$VERSION: ${HEAD_TITLE:-${TITLE:-release}}" \
        -m "Written by tools/ship_release.sh from goh ship's {version}; ./release.sh checks, installs, tags and pushes it."
    trap - EXIT
    ok "release commit $(git rev-parse --short HEAD)"
fi

./release.sh "v$VERSION"
