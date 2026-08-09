# Development Guide

Internal documentation for contributors and maintainers.

## Quick Links

| Topic | Document |
|-------|----------|
| **Release Process** | [RELEASE.md](RELEASE.md) |
| **Performance** | [docs/performance.md](docs/performance.md) |
| **Roadmap** | [docs/roadmap.md](docs/roadmap.md) — the single forward-looking backlog |

## Optional: faster local builds

`sccache` caches compiled crates and cuts rebuild times noticeably on this
workspace. To use it:

```bash
brew install sccache          # or: cargo install sccache
mkdir -p .cargo
printf '[build]\nrustc-wrapper = "sccache"\n' > .cargo/config.toml
```

`.cargo/config.toml` is gitignored on purpose. It was committed once, and
because a `rustc-wrapper` must exist on every machine that builds the project,
CI died before compiling anything:

```
error: could not execute process `sccache .../rustc -vV` (never executed)
Caused by: No such file or directory (os error 2)
```

A build wrapper is a property of the machine, not of the project.

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

Every gate below runs on **every commit** via `.githooks/pre-commit`, and again
in CI (`.github/workflows/ci.yml`) where it cannot be bypassed. The two lists are
deliberately identical: a local gate weaker than CI is a gate that lets you push
red, which is how ten deprecations once shipped green.

| Gate | Command | What it stops |
|------|---------|---------------|
| Gate parity | `python3 tools/check_gate_parity.py` | A gate named in one of the three lists and not the others |
| No emoji | `python3 tools/check_no_emoji.py` | Decorative emoji anywhere, including as unicode escapes |
| Test-only code | `python3 tools/check_test_only_code.py` | A function exercised only by tests, whose live duplicate is then untested |
| Key hints | `python3 tools/check_key_hints.py` | A user-facing string naming a key nothing binds |
| Keychain isolation | `python3 tools/check_keychain_isolation.py` | A test reaching the real OS keychain and stopping the suite on a dialog |
| Row 0 owner | `python3 tools/check_row0_owner.py` | A pane's view assigned outside `panel.rs`, which overwrites the banner |
| Magic numbers | `python3 tools/check_magic_numbers.py` | A literal carrying meaning nobody wrote down |
| Formatting | `cargo fmt --all -- --check` | |
| Clippy | `cargo clippy --workspace --all-targets --all-features -- -D warnings` | |
| Coverage | `bash tools/coverage_check.sh` | Line coverage below 95% |

The three lists -- the hook, `ci.yml` and `local-ci.py` -- are compared against
each other by the first gate, and against what is actually in `tools/`. They had
drifted before: this file itself said "three gates" while CI ran six and
`local-ci.py` ran two. A comment asking people to keep lists in step is not a
gate; that one is.

Each checker has a `--self-test` that proves it still *detects* before it is
trusted to report clean, and the hook runs it first. A checker that has quietly
stopped detecting reports a clean tree exactly like a clean tree does -- the
emoji gate missed escaped codepoints entirely until a padlock turned up in a
repo it called clean.

To run the whole set plus the fuzz and benchmark gates before pushing:

```bash
python3 scripts/local-ci.py
```

### CI runs stable; local development runs nightly

There is no `rust-toolchain.toml`, on purpose -- `cargo fuzz` needs nightly and
the fuzz gate in `local-ci.py` would stop working under a pin. The cost is that
the two run different lint sets, in both directions:

* a lint nightly has and stable does not makes the *name* in an `#[allow]` an
  error on stable, under `-D unknown-lints`;
* and removing the allow to satisfy stable makes the lint itself an error here.

`clippy::unused_async_trait_impl` is exactly that, in three files. Both are
needed until it reaches stable:

```rust
#[allow(unknown_lints)]
#[allow(clippy::unused_async, clippy::unused_async_trait_impl)]
```

If clippy passes here and fails there, suspect the lint *name* before the code.

### Checking the Linux-only code from a Mac

`fprintd.rs`, the `secure_enclave` stubs and the Linux arms of `api/` are
`cfg`-gated away on macOS, so nothing local compiles them and they can rot
silently -- they had, for long enough to drift a whole zbus major version, while
CI was failing earlier for an unrelated reason and never reached them.

CI builds them now, and that is the real gate. To check before pushing, without
waiting for a CI round trip: `cargo check` does not link, so a stub `.pc` is
enough to satisfy the `libdbus-sys` build script.

```bash
rustup target add x86_64-unknown-linux-gnu
printf 'Name: dbus\nDescription: stub\nVersion: 1.14.10\nLibs:\nCflags:\n' > /tmp/pc/dbus-1.pc
PKG_CONFIG_ALLOW_CROSS=1 PKG_CONFIG_PATH=/tmp/pc \
  cargo clippy -p multitop-vault --all-targets --all-features \
  --target x86_64-unknown-linux-gnu -- -D warnings
```

The whole workspace will not cross-compile this way -- some C dependencies need
a Linux toolchain -- but the vault crate is where all the platform-gated code
is, and it does.

Enable the hook after cloning (it is a one-time, per-clone setting):

```bash
git config core.hooksPath .githooks
```

Clippy runs with `--all-targets` on purpose: tests and benches inherit the
workspace lint config, and that is exactly where warnings have accumulated.
Fix warnings properly — do not silence them with `#[allow]`.

### Keychain isolation

An integration test binary is compiled without `cfg(test)`, so
`password_store::is_mock_enabled()` is false unless the test says otherwise, and
anything reaching `password_store` -- including `passwords::open`, via
`Panel::ensure_sudo_password` -- then queries the **real** OS keychain. Every
rebuild changes the binary's code signature, so macOS raises a keychain-access
dialog and the suite stops dead waiting for someone to type their login
password. A test can also overwrite or delete credentials the user depends on.

Divert the store first, and hold the guard for the whole test body:

```rust
let _guard = password_store::lock_for_test();   // lock_for_test_async().await in #[tokio::test]
password_store::enable_mock_store();
password_store::clear_mock_store();
```

Only the Susan Kare icon set is permitted in place of emoji:
`→ · ✓ ✗ ⚠ ↔ ↑ ↓`, plus the Mac key glyphs `← ⌘ ⌥ ⌨`.

## Tests

Three shapes, and the distinction matters:

| Shape | Where | What it proves |
|-------|-------|----------------|
| Unit | `#[cfg(test)]` in `src/` | One function's contract. Compiled with `cfg(test)`, so the credential store is mocked automatically. |
| Integration | `crates/*/tests/` | A path through the real API. Compiled **without** `cfg(test)` — see Keychain isolation below. |
| Keystroke-through-render | `config_panel_e2e.rs`, `filter_e2e.rs` | Real `KeyEvent`s through `run::handle_key`, then a real `ui::draw` into a `TestBackend`. |

That third shape exists because the suite once had thorough coverage of
`handle_key` and `password_actions::apply` and never once ran the renderer, so a
panic in `config_ui::draw` was invisible to all of it. Anything that changes the
Configuration panel or the grid belongs there.

Both of those files end in a sweep that walks every key sequence the panel
accepts to a fixed depth and draws each frame. The class being ruled out is "a
state the UI can reach that the renderer cannot draw" — one reported crash is an
instance of it, and only walking the reachable states covers the class.

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