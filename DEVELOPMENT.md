# Development Guide

Internal documentation for contributors and maintainers.

## Quick Links

| Topic | Document |
|-------|----------|
| **Release Process** | [RELEASE.md](../RELEASE.md) |
| **E2E Test Gaps** | [docs/e2e_test_gaps.md](docs/e2e_test_gaps.md) |
| **E2E Test Phased Plan** | [docs/e2e_test_phased_plan.md](docs/e2e_test_phased_plan.md) |
| **Upgrade E2E Test Plan** | [docs/upgrade_e2e_test_plan.md](docs/upgrade_e2e_test_plan.md) |
| **Performance** | [docs/performance.md](docs/performance.md) |
| **Vault Roadmap** | [docs/vault_roadmap.md](docs/vault_roadmap.md) |
| **Roadmap** | [docs/roadmap.md](docs/roadmap.md) |

## Release Workflow

See [RELEASE.md](../RELEASE.md) for the complete automated release process:

```bash
# Bump version, tag, and run automation
sed -i '' 's/version = "0.21.0"/version = "0.22.0"/' Cargo.toml
git add Cargo.toml && git commit -m "chore: bump version to 0.22.0"
git tag -a v0.22.0 -m "Release 0.22.0"
git push origin v0.22.0
python3 scripts/release.py v0.22.0
```

## Test Commands

```bash
# All tests (single-threaded for mock store)
cargo test --package multitop -- --test-threads=1

# Specific test suites
cargo test --package multitop --test config_e2e_test -- --test-threads=1
cargo test --package multitop --test ssh_e2e_test -- --test-threads=1
cargo test --package multitop --test tasks_e2e_test -- --test-threads=1
cargo test --package multitop --test panel_e2e_test -- --test-threads=1
cargo test --package multitop --test server_settings_test -- --test-threads=1
cargo test --package multitop --lib -- --test-threads=1
```

## Build

```bash
# Local build with embedded agents
./build.sh

# Cross-compile with zigbuild (used in CI/Homebrew)
./build.sh --backend zigbuild
```

## Key Files

| File | Purpose |
|------|---------|
| `scripts/release.py` | Automated release (GitHub + Homebrew) |
| `build.sh` | Build script with agent embedding |
| `Cargo.toml` | Workspace version + dependencies |
| `crates/multitop/Cargo.toml` | Package metadata |
| `config.example.toml` | Sample configuration |