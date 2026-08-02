# Development Guide

Internal documentation for contributors and maintainers.

## Quick Links

| Topic | Document |
|-------|----------|
| **Release Process** | [RELEASE.md](RELEASE.md) |
| **E2E Test Gaps** | [docs/e2e_test_gaps.md](docs/e2e_test_gaps.md) |
| **E2E Test Phased Plan** | [docs/e2e_test_phased_plan.md](docs/e2e_test_phased_plan.md) |
| **Upgrade E2E Test Plan** | [docs/upgrade_e2e_test_plan.md](docs/upgrade_e2e_test_plan.md) |
| **Performance** | [docs/performance.md](docs/performance.md) |
| **Roadmap** | [docs/roadmap.md](docs/roadmap.md) — the single forward-looking backlog |

## Release Workflow

See [RELEASE.md](RELEASE.md) for details. Cutting a release is one command —
it bumps the version, refreshes `Cargo.lock`, commits, tags, pushes, publishes
the GitHub release, and updates the Homebrew tap:

```bash
python3 scripts/release.py v0.23.0 --cut
```

Do not run the steps by hand: that is how `v0.21.0` and `v0.22.0` ended up
tagged but never released, with Homebrew left on `v0.20.10`.

## Commit Gates

Three gates run on every commit via `.githooks/pre-commit`, and again in CI
(`.github/workflows/ci.yml`) where they cannot be bypassed:

| Gate | Command |
|------|---------|
| No emoji | `python3 tools/check_no_emoji.py` |
| Formatting | `cargo fmt --all -- --check` |
| Clippy | `cargo clippy --workspace --all-targets --all-features -- -D warnings` |

Enable the hook after cloning (it is a one-time, per-clone setting):

```bash
git config core.hooksPath .githooks
```

Clippy runs with `--all-targets` on purpose: tests and benches inherit the
workspace lint config, and that is exactly where warnings have accumulated.
Fix warnings properly — do not silence them with `#[allow]`.

Only the Susan Kare icon set is permitted in place of emoji:
`→ · ✓ ✗ ⚠ ↔ ↑ ↓`, plus the Mac key glyphs `← ⌘ ⌥ ⌨`.

## Test Commands

```bash
# Everything
cargo test --workspace --all-features

# A single suite
cargo test --package multitop --test server_settings_test
```

`--test-threads=1` is no longer required. Tests that touch the process-global
mock credential store take a guard (`password_store::lock_for_test`, or
`lock_for_test_async` inside `#[tokio::test]`) for the duration of the test
body, so they serialize against each other without serializing the whole suite.
Hold the guard for the entire test — dropping it early re-opens the race.

## Live SSH Tests

The SSH-backed tests are `#[ignore]`d because they need a real reachable host.
CI never runs them; run them by hand against real machines.

> **Never point a live test at a real upgrade command.** Use a read-only
> stand-in — `ls -l ; ls -l` — so the tests exercise the full SSH, streaming,
> locking, and exit-code paths without touching packages on the target. The
> tests in `upgrade_loop_remote_e2e.rs` already hard-code safe commands
> (`ls -l`, `true`, `echo`, `seq`) and never read your `config.toml`, so they
> cannot pick up the real `upgrade_cmd` by accident. Keep it that way: if you
> add a live test, give it a read-only command.

```bash
# Point at a host from your ~/.config/multitop/config.toml
MULTITOP_TEST_SSH_HOST=<host> \
MULTITOP_TEST_SSH_USER=<user> \
MULTITOP_TEST_SSH_PORT=22 \
  cargo test -p multitop --test upgrade_loop_remote_e2e -- --ignored --test-threads=1
```

`--test-threads=1` *is* needed here: the tests contend on a per-host remote
lock file, and running them concurrently makes them flap between "ran" and
"lock prevented execution".

The live telemetry benchmark needs the Linux agent embedded in the binary, so
run `./build.sh` first or it fails with "No x86_64 agent was built into this
binary":

```bash
./build.sh
BENCH_DURATION_SECS=60 BENCH_REMOTE_HOST=<host> BENCH_REMOTE_USER=<user> \
  cargo bench --bench remote_ssh_bench
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