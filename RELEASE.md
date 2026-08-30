# Release Process

Run the gates first — `python3 scripts/local-ci.py` is everything CI runs, plus
the fuzz and benchmark gates the hook does not carry.

## Prerequisites

- `gh` (GitHub CLI) authenticated: `gh auth login`
- `git` on PATH
- Push access to `ztomer/homebrew-tap`

The tap push needs a token with `repo` scope. The script takes it from
`GITHUB_TOKEN`/`GH_TOKEN` if set, and otherwise falls back to `gh auth token`,
so a plain `gh auth login` (keyring, no env var) is enough.

## Procedure

One command — bump, commit, tag, push, release, and update the tap:

```bash
python3 scripts/release.py v0.32.0 --cut
```

Do not perform these steps by hand. Hand-running them is how `v0.21.0` and
`v0.22.0` ended up tagged but never released, leaving Homebrew serving
`v0.20.10` while the repo claimed two newer versions.

To release a tag that was already pushed, omit `--cut`:

```bash
python3 scripts/release.py v0.32.0
```

## What the script does (`scripts/release.py`)

With `--cut`, first:

0. **Cuts the release** — verifies a clean tree, bumps the workspace version in
   `Cargo.toml`, refreshes `Cargo.lock` so it cannot drift, commits (through the
   pre-commit gates), creates the annotated tag, and pushes branch + tag

Then, always:

1. **Verifies tag** — exists locally and on origin
2. **Builds release notes** — from commits since the last **published release**,
   not the last tag. Tags that were never released must not truncate the notes,
   or users upgrading via Homebrew silently miss everything in between.
3. **Creates GitHub release** — via `gh` CLI
4. **Downloads tarball** — calculates SHA256 from `https://github.com/ztomer/multitop/archive/refs/tags/vX.Y.Z.tar.gz`
5. **Updates Homebrew formula** — clones `ztomer/homebrew-tap`, updates `Formula/multitop.rb` (version + hash), commits and pushes

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
#    - Update url to v0.32.0.tar.gz
#    - Update sha256 to calculated value
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