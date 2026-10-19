#!/usr/bin/env bash
# Cut a multitop release: build the Linux agents, publish them, and bump the
# Homebrew formula (source tarball AND agent resources) — one command so the
# tap can never go stale again.
#
# Usage: ./release.sh v0.48.0 [--dry-run]
#
#   --dry-run skips exactly the world-side steps: no tag, no push, no GitHub
#   release, no tap push, and the digests it shows belong to the locally
#   built artifacts (a release does not exist yet to roundtrip). The tap
#   bump's [dry-run] line names the tap gate step a real run would execute.
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
#      CHANGELOG stanza, rustup musl targets, gh present; --dry-run skips the
#      tag-exists precondition, because a dry run tags nothing)
#   2. gates (make clippy-targets + cargo test, mirroring CI)
#   3. ./build.sh — cross-compiles both agents from this checkout
#   4. tag, push the tag, create the GitHub release with CHANGELOG notes,
#      upload both agent binaries
#   5. bump Formula/multitop.rb in ztomer/homebrew-tap THROUGH THE TAP'S OWN
#      GATE (G3): one call to goh's tools/release-kit/tap_bump.sh -- the
#      version moves together with the tarball url/sha AND both agent
#      resource urls/shas (its --resource flag), the tap's gate runs in its
#      clone, and the commit is pushed pinned
#   6. prove it: brew update the tap and fetch the formula
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "${SCRIPT_DIR}"

DRY_RUN=0
TAG=""
for arg in "$@"; do
  case "${arg}" in
    --dry-run) DRY_RUN=1 ;;
    v[0-9]*.[0-9]*.[0-9]*) TAG="${arg}" ;;
    *) echo "error: unknown argument '${arg}' (usage: ./release.sh vMAJOR.MINOR.PATCH [--dry-run])" >&2; exit 1 ;;
  esac
done
[ -n "${TAG}" ] || { echo "error: usage: ./release.sh vMAJOR.MINOR.PATCH [--dry-run]" >&2; exit 1; }
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
installed_targets="$(rustup target list --installed 2>/dev/null || true)"
for t in ${TRIPLES}; do
  grep -qx "${t}" <<< "${installed_targets}" \
    || die "musl target ${t} missing — run: rustup target add ${t}"
done

git diff --quiet || die "working tree is dirty — commit or stash first"
git diff --cached --quiet || die "staged changes present — commit first"
[ "$(git branch --show-current)" = "main" ] || die "not on main"
# The bump commit may be unpushed: it and the tag go to origin in ONE atomic
# push below. What must hold is that HEAD descends from origin/main, or that
# push would be a rewrite.
git fetch -q origin main || die "cannot fetch origin/main"
git merge-base --is-ancestor origin/main HEAD || die "HEAD does not descend from origin/main — rebase first"
if [ "${DRY_RUN}" = 1 ]; then
  info "[dry-run] no tag will be created or pushed; a tag may already exist"
else
  git rev-parse "${TAG}" >/dev/null 2>&1 && die "tag ${TAG} already exists"
fi

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

if [ "${DRY_RUN}" = 1 ]; then
  ok "[dry-run] cross-compile shown viable; skipping tag, push and release"
else
  info "tagging ${TAG} ..."
  git tag -a "${TAG}" -m "Release ${TAG}"
  # Branch and tag in one atomic push: both refs move or neither, and the
  # pre-push gate runs once over the commit both name.
  git push --atomic origin HEAD:main "${TAG}" || {
    git tag -d "${TAG}" >/dev/null
    die "the push of main and ${TAG} failed — nothing moved on origin; the local tag is removed"
  }
  head_sha="$(git rev-parse HEAD)"
  remote="$(git ls-remote origin refs/heads/main "refs/tags/${TAG}^{}")"
  grep -q "^${head_sha}[[:space:]]refs/heads/main$" <<< "${remote}" || die "origin's main is not ${head_sha}"
  grep -q "^${head_sha}[[:space:]]refs/tags/${TAG}^{}$" <<< "${remote}" || die "origin's ${TAG} does not name ${head_sha}"
  ok "pushed main and ${TAG} together; origin has both at ${head_sha}"

  info "creating the GitHub release ..."
  NOTES="$(mktemp)"
  trap 'rm -rf "${AGENTS}" "${NOTES}" "${DL:-}"' EXIT
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
fi

info "computing the new shas (downloads = roundtrip proof; a --dry-run digests the local artifacts instead) ..."
DL="$(mktemp -d)"
trap 'rm -rf "${AGENTS}" "${DL:-}"' EXIT
if [ "${DRY_RUN}" = 1 ]; then
    # No release exists on the CDN yet, so there is nothing to roundtrip: a
    # dry run digests the locally built artifacts. The downloads are the
    # proof that shipped bytes equal built bytes -- that proof belongs to the
    # real run. git archive carries the working HEAD with the prefix the
    # real tarball's URL implies.
    git archive --format=tar.gz --prefix="multitop-${VER}/" -o "${DL}/tarball.tar.gz" HEAD
    TARBALL="${DL}/tarball.tar.gz"
    AGENT_X64="${AGENTS}/multitop-agent-x86_64-unknown-linux-musl"
    AGENT_ARM="${AGENTS}/multitop-agent-aarch64-unknown-linux-musl"
    TARBALL_SHA="$(shasum -a 256 "${TARBALL}" | cut -d' ' -f1)"
    SHA_X64="$(shasum -a 256 "${AGENT_X64}" | cut -d' ' -f1)"
    SHA_ARM="$(shasum -a 256 "${AGENT_ARM}" | cut -d' ' -f1)"
else
    # The release was created seconds ago; a 404/empty body from the CDN here
    # silently produced empty shas once, which the transform then misdiagnosed as
    # a formula layout bug. Fail on the real cause: HTTP errors, empty bodies, or
    # a digest that is not 64 hex digits.
    fetch_sha() {
        local url="$1" out="$2"
        curl -fsSL --retry 3 --retry-delay 2 "${url}" -o "${out}" \
            || die "failed to download ${url}"
        [ -s "${out}" ] || die "downloaded ${url} is empty"
        shasum -a 256 "${out}" | cut -d' ' -f1
    }
    TARBALL_SHA="$(fetch_sha "https://github.com/${REPO}/archive/refs/tags/${TAG}.tar.gz" "${DL}/tarball.tar.gz")"
    SHA_X64="$(fetch_sha "https://github.com/${REPO}/releases/download/${TAG}/multitop-agent-x86_64-unknown-linux-musl" "${DL}/agent-x64")"
    SHA_ARM="$(fetch_sha "https://github.com/${REPO}/releases/download/${TAG}/multitop-agent-aarch64-unknown-linux-musl" "${DL}/agent-arm")"
    TARBALL="${DL}/tarball.tar.gz"
    AGENT_X64="${DL}/agent-x64"
    AGENT_ARM="${DL}/agent-arm"
fi
for v in "${TARBALL_SHA}" "${SHA_X64}" "${SHA_ARM}"; do
    [[ "${v}" =~ ^[0-9a-f]{64}$ ]] \
        || die "sha256 shape check failed: '${v}'"
done

# ── the tap bump, through the tap's own gate (G3, goh 6.8) ─────────────────
# One call, several artifacts: tap_rewrite.py reads the formula as the DSL it
# is and gives every url and sha256 its own artifact's digest -- the shape
# tools/bump_formula.py encrusted (deleted with this change; two mechanisms
# for one bump is how v0.49.0 shipped stale agent urls). The tap's own
# `tools/gate.sh --full` runs on the bump and the commit is pushed pinned --
# a red gate pushes nothing. --dry-run names those steps and pushes nothing.
info "bumping ${TAP}/${FORMULA_PATH} via tap_bump.sh → ${TAG} ..."
TAP_BUMP_ARGS=(--tap "${TAP}" --formula multitop --tag "${TAG}"
  --artifact "${TARBALL}"
  --resource "multitop-agent-x86_64=${AGENT_X64}"
  --resource "multitop-agent-aarch64=${AGENT_ARM}")
if [ "${DRY_RUN}" = 1 ]; then
  TAP_BUMP_ARGS+=(--dry-run)
fi
"${GOH_DIR:-$HOME/Projects/gates_of_heck}/tools/release-kit/tap_bump.sh" "${TAP_BUMP_ARGS[@]}"
ok "formula bump handed to tap_bump.sh"

if [ "${DRY_RUN}" = 1 ]; then
  ok "[dry-run] stopped short: nothing tagged, released, bumped, or pushed"
  exit 0
fi

info "proving it: brew update + fetch ..."
ok "formula bump handed to tap_bump.sh"

if [ "${DRY_RUN}" = 1 ]; then
  ok "[dry-run] stopped short: nothing tagged, released, bumped, or pushed"
  exit 0
fi

info "proving it: brew update + fetch ..."
brew update >/dev/null 2>&1
brew fetch "ztomer/tap/multitop" --force >/dev/null 2>&1 || die "brew fetch failed after tap bump"
ok "released ${TAG}: tarball + both agent resources verified through Homebrew"
