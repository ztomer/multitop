# Release Process

Run the gates first — `tools/gate.sh --full` is the gate of record, and it is
the same list the pre-push hook and CI run. `tools/check_gate_parity.py` holds
all three to one glob, so a gate cannot be added to one of them and missed in
the others.

(`scripts/local-ci.py` was this file's instruction until 2026-09-14, when it was
deleted. It was a 481-line orchestrator with its own colours, its own fan-out and
a second copy of the checker list — the same duplication that killed
`tools/repo_gates.sh`.)

## Prerequisites

- `gh` (GitHub CLI) authenticated: `gh auth login`
- `git` on PATH
- Push access to `ztomer/homebrew-tap`
- Optional connectivity and auth verification: `./tools/push_probe.sh`

The tap push needs a token with `repo` scope. The script takes it from
`GITHUB_TOKEN`/`GH_TOKEN` if set, and otherwise falls back to `gh auth token`,
so a plain `gh auth login` (keyring, no env var) is enough.

## Procedure

### Recommended: `./release.sh`
The primary release tool is `./release.sh`. It gates, cross-compiles both Linux agents, tags, pushes, creates the GitHub release with agent binary assets attached, bumps `Formula/multitop.rb` in `ztomer/homebrew-tap` (both source tarball and agent resources), and verifies the formula via `brew update` and `brew fetch`:

```bash
# 1. Write the CHANGELOG stanza (release.sh writes the version and lockfiles)
# 2. Run the release:
./release.sh v0.49.1
```

`release.sh` also does the bump itself, so the two manual steps above are
optional — pass the tag and it will write the version, the lockfiles and the
CHANGELOG stanza, commit them through the pre-commit gates, and carry on to the
tag, the release and the tap.

There was a second implementation, `scripts/release.py` (429 lines), offered
here as an "alternative" until 2026-09-28. It was deleted rather than fixed,
and the reason is worth keeping: it predated the change that made the formula
carry the two agent binaries as resources, so it rewrote the Homebrew formula
with `re.sub(r'sha256 "[^"]*"', ...)` and no count — which sets **every**
`sha256` line in the file, tarball and both agents, to the tarball's digest. It
had no concept of the agent urls at all. Shipping it would have produced a
formula that installs and then fails every agent checksum, and the version that
documented it as supported is the version a release would have been cut with.

That is the same failure as the retired `local-ci.py` and the retired `tools/repo_gates.sh`: a second
copy of a list, drifting. The rule this repo now holds to is one implementation
per job, and the fix is to delete the copy rather than to keep both in step.

## What the script does (`release.sh`)

1. **Verifies preconditions** — clean tree, on `main`, `main` pushed, tag not
   already existing, workspace `Cargo.toml` version equal to the tag, the
   CHANGELOG stanza present, musl targets installed, `gh` present
2. **Bumps and commits** — workspace version in `Cargo.toml`, `Cargo.lock`
   refreshed so it cannot drift, the agents rebuilt with the new version baked
   in (every bump stales them by definition, and the agent-version gate would
   refuse the commit without it), and `fuzz/Cargo.lock` too
3. **Tags and pushes** — annotated tag, branch, tag
4. **Verifies tag** — exists locally and on origin
5. **Builds release notes** — from commits since the last **published release**,
   not the last tag. Tags that were never released must not truncate the notes,
   or users upgrading via Homebrew silently miss everything in between.
6. **Creates the GitHub release** — via `gh`, with both agent binaries attached
   as assets
7. **Updates the Homebrew formula** — via the `gh` contents API (no clone, so
   there is no second git credential in play), through `tools/bump_formula.py`:
   source tarball url + digest **and** both agent resource urls + digests
8. **Proves the formula** — `brew update` then `brew fetch`, and fails if
   Homebrew will not resolve it

## Post-release

Users can install/upgrade via:

```bash
brew upgrade ztomer/tap/multitop
# or
brew install ztomer/tap/multitop
```

## Manual fallback (if script fails)

```bash
# 1. Get SHA256
curl -sL https://github.com/ztomer/multitop/archive/refs/tags/v0.32.0.tar.gz | shasum -a 256

# 2. Update homebrew-tap/Formula/multitop.rb
#    - Update the tarball url to vX.Y.Z.tar.gz and its sha256 to the value above
#    - Update BOTH agent resource urls to vX.Y.Z and both their sha256 lines
#      (amd64 + aarch64). Pair each digest with the url above it, never by line
#      order: the v0.49.0 formula shipped 0.49.0 digests beside 0.47.3 urls and
#      Homebrew refused it outright.
#    - Commit and push
```

## Notes

- **Check for tags that were never released.** `gh release list` against
  `git tag` is worth a glance before cutting: v0.37.0, v0.40.0, v0.41.0 and
  v0.42.x were all tagged and pushed without ever being released, so Homebrew
  served v0.39.1 while the repo claimed 0.42.1. This is the failure the
  procedure above already warns about, and it happened four more times. The
  notes are built from the last **published** release for exactly this reason,
  so one good release absorbs the gap.
- **Pushing the tag does not re-run the gates.** The commit it names is already
  on the remote and was gated to get there. Before that, cutting a release ran
  the full suite four times -- pre-flight, version-bump commit, branch push, tag
  push -- and the tag one is the one that hit a timeout and left v0.43.0
  half-released.
- **Both lockfiles are refreshed.** `fuzz/` is outside the workspace and carries
  its own `Cargo.lock` recording the workspace crates by version, so a bump left
  it naming the previous release until something happened to build a fuzz
  target. It then turns up as an unexplained dirty file mid-release.
- Script is **idempotent** — safe to re-run
- Requires `GITHUB_TOKEN` with push access to `ztomer/homebrew-tap`
- Homebrew formula lives in separate repo: `~/Projects/homebrew-tap` (or `gh repo clone ztomer/homebrew-tap`)
- Tag format must be `vX.Y.Z` (semantic versioning)