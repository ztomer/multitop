#!/usr/bin/env bash
# Cut a multitop release: build the Linux agents, publish them, and bump the
# Homebrew formula (source tarball AND agent resources) — one command so the
# tap can never go stale again.
#
# Usage: ./release.sh v0.48.0
#
# The version bump itself stays a separate commit made beforehand: the
# workspace Cargo.toml must already carry the tag's version, CHANGELOG.md
# must have a stanza for it, and the tree must be clean, so the tag points
# at exactly what was tested.
#
# Why the formula needs agents as release assets: the brew sandbox has no
# musl target std (and no network to fetch it), so the agents cannot be
# cross-compiled during `brew install`. Formula Casks/multitop.rb therefore
# consumes prebuilt agents via `resource` stanzas, and build.rs hard-gates
# that an embedded agent reports the workspace version — so the assets MUST
# be built from the tagged commit, and the tap bump MUST move the resource
# URLs + shas in lockstep with the version (a version-only bump embeds stale
# agents and the build fails the gate).
#
# What it does:
#   1. preconditions (clean tree, on main, main pushed, version match,
#      CHANGELOG stanza, rustup musl targets, gh present)
#   2. gates (make clippy-targets + cargo test, mirroring CI)
#   3. ./build.sh — cross-compiles both agents from this checkout
#   4. tag, push the tag, create the GitHub release with CHANGELOG notes,
#      upload both agent binaries
#   5. bump Formula/multitop.rb in ztomer/homebrew-tap via the gh API:
#      source tarball url + sha AND both resource urls + shas
#   6. prove it: brew update the tap and fetch the formula
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "${SCRIPT_DIR}"

TAG="${1:?Usage: ./release.sh vX.Y.Z}"
case "${TAG}" in
  v[0-9]*.[0-9]*.[0-9]*) ;;
  *) echo "error: tag must look like vMAJOR.MINOR.PATCH (got '${TAG}')" >&2; exit 1 ;;
esac
VER="${TAG#v}"

REPO="ztomer/multitop"
TAP="ztomer/homebrew-tap"
FORMULA_PATH="Formula/multitop.rb"
TRIPLES="x86_64-unknown-linux-musl aarch64-unknown-linux-musl"

die() { echo "error: $*" >&2; exit 1; }
info() { echo "→ $*"; }
ok() { echo "✓ $*"; }

command -v gh >/dev/null || die "gh CLI required (brew install gh)"
command -v cargo >/dev/null || die "cargo required"
command -v rustup >/dev/null || die "rustup required (agents need musl targets)"
for t in ${TRIPLES}; do
  rustup target list --installed 2>/dev/null | grep -qx "${t}" \
    || die "musl target ${t} missing — run: rustup target add ${t}"
done

git diff --quiet || die "working tree is dirty — commit or stash first"
git diff --cached --quiet || die "staged changes present — commit first"
[ "$(git branch --show-current)" = "main" ] || die "not on main"
[ -z "$(git log --oneline origin/main..HEAD 2>/dev/null)" ] || die "main has unpushed commits — push first"
git rev-parse "${TAG}" >/dev/null 2>&1 && die "tag ${TAG} already exists"

WS_V="$(grep -m1 '^version = ' Cargo.toml | cut -d'"' -f2)"
[ "${WS_V}" = "${VER}" ] || die "workspace Cargo.toml is ${WS_V}, tag wants ${VER} — bump it in a commit first"
grep -q "^## v${VER}\\b" CHANGELOG.md || die "CHANGELOG.md has no stanza for v${VER} — add it first"

info "running the gates (mirrors CI) ..."
make clippy-targets >/dev/null || die "clippy-targets failed — nothing tagged"
cargo test --workspace >/dev/null 2>&1 || die "cargo test failed — nothing tagged"
ok "gates green"

info "cross-compiling the agents ..."
./build.sh

TARGET_DIR="$(cargo metadata --format-version 1 2>/dev/null | python3 -c 'import json, sys; print(json.load(sys.stdin)["target_directory"])')"
AGENTS="$(mktemp -d)"
trap 'rm -rf "${AGENTS}"' EXIT
for t in ${TRIPLES}; do
  src="${TARGET_DIR}/${t}/release/multitop-agent"
  [ -f "${src}" ] || die "agent missing at ${src} after build.sh"
  grep -qa "${VER}" "${src}" || die "agent ${t} does not embed version ${VER} — refusing to ship stale bytes"
  cp "${src}" "${AGENTS}/multitop-agent-${t}"
  ok "${t} $(wc -c < "${src}" | tr -d ' ') bytes, embeds ${VER}"
done

info "tagging ${TAG} ..."
git tag -a "${TAG}" -m "Release ${TAG}"
git push origin "${TAG}"
ok "pushed ${TAG}"

info "creating the GitHub release ..."
NOTES="$(mktemp)"
trap 'rm -rf "${AGENTS}" "${NOTES}"' EXIT
python3 - "${VER}" CHANGELOG.md > "${NOTES}" <<'PY'
import re, sys
ver = sys.argv[1]
text = open(sys.argv[2]).read()
m = re.search(r'^## v%s\b.*?(?=^## v\d|\Z)' % re.escape(ver), text, re.M | re.S)
sys.stdout.write(m.group(0).strip() if m else 'multitop %s' % ver)
PY
gh release create "${TAG}" \
  "${AGENTS}/multitop-agent-x86_64-unknown-linux-musl" \
  "${AGENTS}/multitop-agent-aarch64-unknown-linux-musl" \
  --repo "${REPO}" --title "${TAG}" --notes-file "${NOTES}"
ok "release published with agent assets"

info "computing the new shas (downloads = roundtrip proof) ..."
TARBALL_SHA="$(curl -sL "https://github.com/${REPO}/archive/refs/tags/${TAG}.tar.gz" | shasum -a 256 | cut -d' ' -f1)"
SHA_X64="$(curl -sL "https://github.com/${REPO}/releases/download/${TAG}/multitop-agent-x86_64-unknown-linux-musl" | shasum -a 256 | cut -d' ' -f1)"
SHA_ARM="$(curl -sL "https://github.com/${REPO}/releases/download/${TAG}/multitop-agent-aarch64-unknown-linux-musl" | shasum -a 256 | cut -d' ' -f1)"

info "bumping ${TAP}/${FORMULA_PATH} → ${VER} (tarball + both resources) ..."
CUR_SHA="$(gh api "repos/${TAP}/contents/${FORMULA_PATH}" --jq .sha)"
CUR_FORMULA="$(gh api "repos/${TAP}/contents/${FORMULA_PATH}" --jq .content | base64 --decode)"
NEW_FORMULA="$(printf '%s' "${CUR_FORMULA}" | NEW_TAG="${TAG}" TARBALL_SHA="${TARBALL_SHA}" SHA_X64="${SHA_X64}" SHA_ARM="${SHA_ARM}" python3 -c "
import os, re, sys
s = sys.stdin.read()
new_tag = os.environ['NEW_TAG']
old = re.search(r'multitop/archive/refs/tags/(v[\d.]+)\.tar\.gz', s)
assert old, 'source tarball url not found'
old_tag = old.group(1)
s = s.replace('multitop/archive/refs/tags/%s.tar.gz' % old_tag,
              'multitop/archive/refs/tags/%s.tar.gz' % new_tag)
s = re.sub(r'sha256 "[0-9a-f]+"', 'sha256 "' + os.environ['TARBALL_SHA'] + '"', s, count=1)
lines = s.split('\n')
out, pending = [], None
for line in lines:
    # Anchor on releases/download/ so ENV[...] filenames and comments below
    # (same strings, no version, no following sha) are never touched.
    if 'releases/download/v' in line and 'multitop-agent-x86_64-unknown-linux-musl' in line:
        line = line.replace('/%s/' % old_tag, '/%s/' % new_tag)
        pending = os.environ['SHA_X64']
    elif 'releases/download/v' in line and 'multitop-agent-aarch64-unknown-linux-musl' in line:
        line = line.replace('/%s/' % old_tag, '/%s/' % new_tag)
        pending = os.environ['SHA_ARM']
    elif pending and re.search(r'sha256 "[0-9a-f]+"', line):
        line = re.sub(r'sha256 "[0-9a-f]+"', 'sha256 "' + pending + '"', line, count=1)
        pending = None
    out.append(line)
assert pending is None, 'agent sha line not found after agent url'
s = '\n'.join(out)
# Command substitution strips trailing newlines; the formula must end with one.
if not s.endswith('\n'):
    s += '\n'
assert old_tag not in [l for l in out if 'multitop-agent-' in l or 'archive/refs/tags' in l], 'stale tag left behind'
sys.stdout.write(s)
")"
if [ "${NEW_FORMULA}" = "${CUR_FORMULA}" ]; then
  die "tap transform produced no change — refusing to push an empty bump"
fi
printf '%s' "${NEW_FORMULA}" | ruby -c >/dev/null || die "transformed formula failed ruby -c"
gh api -X PUT "repos/${TAP}/contents/${FORMULA_PATH}" \
  -f message="multitop ${VER}" \
  -f content="$(printf '%s' "${NEW_FORMULA}" | base64 | tr -d '\n')" \
  -f sha="${CUR_SHA}" >/dev/null
ok "formula updated"

info "proving it: brew update + fetch ..."
brew update >/dev/null 2>&1
brew fetch "ztomer/tap/multitop" --force >/dev/null 2>&1 || die "brew fetch failed after tap bump"
ok "released ${TAG}: tarball + both agent resources verified through Homebrew"
