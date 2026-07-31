# Release Process

## Prerequisites

- `gh` (GitHub CLI) authenticated: `gh auth login`
- `GITHUB_TOKEN` or `GH_TOKEN` environment variable (for pushing to homebrew-tap)
- `git`, `sha256sum` on PATH

## Procedure

```bash
# 1. Bump version in Cargo.toml (workspace root)
sed -i '' 's/version = "0.21.0"/version = "0.22.0"/' Cargo.toml
git add Cargo.toml && git commit -m "chore: bump version to 0.22.0"

# 2. Create and push annotated tag
git tag -a v0.22.0 -m "Release 0.22.0"
git push origin v0.22.0

# 3. Run automated release script
python3 scripts/release.py v0.22.0
```

## What the script does (`scripts/release.py`)

1. **Verifies tag** — exists locally and on origin
2. **Builds release notes** — from git commits since previous tag
3. **Creates GitHub release** — via `gh` CLI with auto-generated notes
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
curl -sL https://github.com/ztomer/multitop/archive/refs/tags/v0.22.0.tar.gz | shasum -a 256

# 2. Update homebrew-tap/Formula/multitop.rb
#    - Update url to v0.22.0.tar.gz
#    - Update sha256 to calculated value
#    - Commit and push
```

## Notes

- Script is **idempotent** — safe to re-run
- Requires `GITHUB_TOKEN` with push access to `ztomer/homebrew-tap`
- Homebrew formula lives in separate repo: `~/Projects/homebrew-tap` (or `gh repo clone ztomer/homebrew-tap`)
- Tag format must be `vX.Y.Z` (semantic versioning)