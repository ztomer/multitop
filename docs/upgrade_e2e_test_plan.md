# Upgrade Loop E2E Regression Test Plan

## Objective

Deterministic end-to-end regression tests covering the full upgrade event loop:
**config → App → confirm_upgrade → spawn_upgrade → shell execution → AuxBegin/AuxLine/AuxDone → state persisted**

Split into two tiers:
1. **Local tests** (deterministic, no SSH) — run on every `cargo test`
2. **Remote tests** (real SSH) — `#[ignore]`, run against configured hosts

## Files

- Local tests: `crates/multitop/tests/upgrade_loop_e2e.rs`
- Remote tests: `crates/multitop/tests/upgrade_loop_remote_e2e.rs`

---

## Local Tests

### Deterministic Strategy

- **Server host:** `127.0.0.1` (triggers local shell path, no SSH required)
- **Upgrade command:** `ls -l ; ls -l` (deterministic output on any POSIX system)
- **Keychain:** mock auto-enabled via `cfg!(test)`
- **State isolation:** `tempfile::TempDir` for config/state persistence assertions
- **Output capture:** `tokio::sync::mpsc` channel to collect all `Msg` variants from `spawn_upgrade`

### Test 1: `test_upgrade_single_server_streams_exact_output`

- Server with `upgrade_cmd: "ls -l ; ls -l"`
- Call `app.run_upgrade()` → get `Command::RunUpgrade`
- Spawn upgrade via `spawn_upgrade(panel, gen, server, None, tx)`
- Collect all `Msg` from channel

**Assertions:**
- `AuxBegin` received with correct panel/gen
- Multiple `AuxLine` containing `total` and file entries (from `ls -l`)
- `AuxDone` with `success: true`
- Output contains at least 20 lines (two `ls -l` runs produce ~20+ lines)

### Test 2: `test_upgrade_multi_server_concurrent_output`

- 3 servers, each with `upgrade_cmd: "echo UPGRADE_{N} && ls -l"`
- All 3 upgrades launched simultaneously via separate `spawn_upgrade` calls
- Collect from one merged channel

**Assertions:**
- All 3 `AuxBegin`/`AuxDone` pairs received
- Each server's output contains correct `UPGRADE_0`/`UPGRADE_1`/`UPGRADE_2` prefix
- No cross-contamination between panels

### Test 3: `test_upgrade_failure_reports_nonzero_exit`

- Server with `upgrade_cmd: "ls -l ; exit 1"`

**Assertions:**
- `AuxDone { success: false }`
- Error note or stderr present in output

### Test 4: `test_upgrade_state_machine_roundtrip`

- Server with `upgrade_cmd: "ls -l"`
- Run `confirm_upgrade()` → get cmds
- Apply `AuxBegin`, several `AuxLine`, `AuxDone` messages to `App`

**Assertions:**
- `panel.mode == Mode::Upgrade` during execution
- `state.toml` written with `upgrade_started_at` and `last_update` timestamps
- After switching back to monitor mode, upgrade output persists in `last_upgrade`

### Test 5: `test_upgrade_vault_password_preloaded`

- Vault with stored password for server
- Run `run_upgrade()` → verify `panel.sudo_password` populated from vault
- Spawn upgrade, verify command executes (password available for sudo)

**Assertions:**
- `panel.sudo_password == Some("...")` after `run_upgrade()`
- Upgrade completes successfully

### Test 6: `test_upgrade_lock_prevents_concurrent`

- Server with `upgrade_cmd: "mkdir -p ~/.cache/multitop && ls -l"`
- Spawn first upgrade, wait briefly for lock acquisition
- Spawn second upgrade immediately

**Assertions:**
- Second upgrade gets "Upgrade already in progress" stderr
- Second `AuxDone` has `success: false`

### Test 7: `test_upgrade_empty_output_handled`

- Server with `upgrade_cmd: "true"` (exits 0, no output)

**Assertions:**
- `AuxBegin` + `AuxDone { success: true }`
- Zero `AuxLine` messages

### Test 8: `test_upgrade_carriage_return_cleaned`

- Server with `upgrade_cmd: "printf 'step1\rstep2\rstep3\n'"`

**Assertions:**
- Each `AuxLine` is clean (no `\r` characters)
- Lines appear as `step1`, `step2`, `step3`

### Test 9: `test_upgrade_generation_staleness`

- Run upgrade, get gen G1
- Start second upgrade (new gen G2)
- Deliver `AuxLine` with gen G1 after G2 started

**Assertions:**
- Stale message (gen G1) dropped, not visible in panel view
- Only G2 messages appear

### Test 10: `test_upgrade_state_persists_across_app_restart`

- `TempDir` for config/state
- Run upgrade, assert `state.toml` written
- Create new `App` with same `config_path`

**Assertions:**
- `state.toml` file exists and is valid
- `last_update` loaded correctly on App restart

---

## UI Cycle Tests

These tests cover the user interaction cycle: navigating to a pane, pressing `u`, returning, and re-pressing `u`.

### Test 11: `test_ui_upgrade_then_return_shows_last_result`

- 2 servers, both with `upgrade_cmd: "ls -l"`
- Focus panel 0, press `u` → upgrade modal → confirm → `run_upgrade` → spawn upgrade
- Apply `AuxBegin`/`AuxLine`/`AuxDone` to App for panel 0
- Switch to panel 1 (`App::switch_stats()`)
- Return to panel 0

**Assertions:**
- Panel 0 view contains the upgrade output lines (persisted in `last_upgrade`)
- Panel 1 shows its own monitor output (unaffected)

### Test 12: `test_ui_second_u_in_upgrade_mode_reinitiates_upgrade`

- 1 server with `upgrade_cmd: "ls -l"`
- First upgrade cycle: press `u` → confirm → upgrade completes → `AuxDone` (panel mode stays `Upgrade`)
- Press `u` again WITHOUT switching to monitor mode

**Assertions:**
- Second press generates `Command::RunUpgrade` with a new gen
- `app.in_upgrade()` is true (mode still Upgrade)
- New `AuxBegin`/`AuxDone` received
- New output streams in

### Test 13: `test_ui_second_u_while_upgrade_in_flight_is_noop`

- 1 server with `upgrade_cmd: "sleep 5 && ls -l"`
- Press `u` → confirm → upgrade starts (still running)
- Press `u` again immediately

**Assertions:**
- `app.upgrades_in_flight()` returns `true`
- Second `u` press produces no `Command::RunUpgrade` (no-op)
- No panic, no duplicate task spawned

### Test 14: `test_ui_vault_locked_shows_prompt_not_modal`

- Server with `upgrade_cmd: "ls -l"`, vault configured but locked
- Press `u`

**Assertions:**
- `show_upgrade_modal` is `false`
- `show_vault_password_prompt` is `true`
- No `Command::RunUpgrade` generated

### Test 15: `test_ui_vault_unlocked_after_password_runs_upgrade`

- Server with `upgrade_cmd: "ls -l"`, vault configured
- Set `vault_unlocked` manually (simulating password entry)
- Press `u`

**Assertions:**
- `show_vault_password_prompt` is `false`
- `show_upgrade_modal` is `true`
- After confirm, `Command::RunUpgrade` generated with vault password preloaded

### Test 16: `test_ui_upgrade_modal_confirmation_flow`

- Server with `upgrade_cmd: "ls -l"`
- Press `u` → modal shown
- Call `confirm_upgrade()`

**Assertions:**
- `show_upgrade_modal` set to `false`
- `upgrade_started_at` timestamp recorded
- `last_update` in state.toml updated after `AuxDone`
- `last_upgrade` buffer populated with output

### Test 17: `test_ui_switching_panes_during_upgrade_preserves_task`

- 3 servers, all with `upgrade_cmd: "sleep 3 && ls -l"`
- Press `u` → confirm → all 3 upgrades start
- Quickly switch to different pane and back while upgrades still running

**Assertions:**
- `tasks.aux_is_upgrade[idx]` is `true` for all panels
- Upgrade tasks NOT cancelled (no early `AuxDone`)
- Switching back shows correct panel view (upgrade running message)

### Test 18: `test_ui_no_upgrade_cmd_shows_message_without_command`

- Server with `upgrade_cmd: None`
- Press `u`

**Assertions:**
- No `Command::RunUpgrade` generated
- Panel view shows "No upgrade_cmd configured"
- `show_upgrade_modal` is `true`

### Test 19: `test_ui_upgrade_output_persists_across_view_switches`

- Server with `upgrade_cmd: "ls -l"`
- Complete full upgrade cycle (press `u`, confirm, `AuxBegin`/`AuxLine`/`AuxDone`)
- Switch to monitor mode, then back to upgrade view

**Assertions:**
- Panel view still shows completed upgrade output
- Output is not cleared when switching views
- `last_upgrade` buffer intact

### Test 20: `test_ui_returning_to_completed_upgrade_shows_output`

- Server with `upgrade_cmd: "ls -l"`
- Complete full upgrade cycle (press `u`, confirm, `AuxBegin`/`AuxLine`/`AuxDone`)
- `switch_stats()` to monitor mode
- Press `u` again

**Assertions:**
- `show_upgrade_output()` called — shows last upgrade output, does NOT start new upgrade
- `Command::RunUpgrade` NOT in returned commands
- Panel view shows `last_upgrade` content

### Test 21: `test_ui_u_during_flight_after_switching_away_is_noop`

- Server with `upgrade_cmd: "sleep 3 && ls -l"`
- Press `u` → confirm → upgrade starts (`upgrade_state = STARTED`, mode = Upgrade)
- `switch_stats()` to monitor mode (`upgrade_state` still `STARTED`, mode = Monitor)
- Press `u` immediately

**Assertions:**
- `upgrades_in_flight()` returns `true` (upgrade_state still STARTED)
- Second `u` is a no-op — no `Command::RunUpgrade`
- No panic, upgrade task continues in background
- When `AuxDone` arrives later, it is still applied (gen matches)

---

## UI Cycle Test Summary

| # | Test Name | Key Check |
|---|-----------|-----------|
| 11 | upgrade_then_return_shows_last_result | Output persists after view switch |
| 12 | second_u_in_upgrade_mode_reinitiates | New gen, new Command::RunUpgrade |
| 13 | second_u_while_in_flight_is_noop | No new command, no panic |
| 14 | vault_locked_shows_prompt | Password prompt, not upgrade modal |
| 15 | vault_unlocked_after_password_runs | Modal shown, password preloaded |
| 16 | modal_confirmation_flow | Timestamp + last_update in state.toml |
| 17 | switching_panes_during_upgrade | Tasks not cancelled, view correct |
| 18 | no_upgrade_cmd_shows_message | No command, "No upgrade_cmd" message |
| 19 | output_persists_across_view_switches | Output not cleared |
| 20 | returning_to_completed_shows_output | Shows last result, not rerun |
| 21 | u_during_flight_after_switching_away | Noop (in_flight), task continues |

---

## Local Test Summary

| # | Test Name | Key Deterministic Check |
|---|-----------|------------------------|
| 1 | single_server_streams_exact_output | ≥20 lines, contains `total`, file perms pattern |
| 2 | multi_server_concurrent_output | 3 distinct `UPGRADE_{N}` prefixes |
| 3 | failure_reports_nonzero_exit | `success == false` on AuxDone |
| 4 | state_machine_roundtrip | state.toml has `upgrade_started_at` + `last_update` |
| 5 | vault_password_preloaded | `sudo_password == Some(...)` after run_upgrade |
| 6 | lock_prevents_concurrent | Second AuxDone success == false |
| 7 | empty_output_handled | Zero AuxLine messages |
| 8 | carriage_return_cleaned | No `\r` in any AuxLine |
| 9 | generation_staleness | Stale gen messages not in panel view |
| 10 | state_persists_across_restart | state.toml survives App drop + recreate |
| 11 | upgrade_then_return_shows_last_result | Output persists after view switch |
| 12 | second_u_in_upgrade_mode_reinitiates | New gen, new Command::RunUpgrade |
| 13 | second_u_while_in_flight_is_noop | No new command, no panic |
| 14 | vault_locked_shows_prompt | Password prompt, not upgrade modal |
| 15 | vault_unlocked_after_password_runs | Modal shown, password preloaded |
| 16 | modal_confirmation_flow | Timestamp + last_update in state.toml |
| 17 | switching_panes_during_upgrade | Tasks not cancelled, view correct |
| 18 | no_upgrade_cmd_shows_message | No command, "No upgrade_cmd" message |
| 19 | output_persists_across_view_switches | Output not cleared |
| 20 | returning_to_completed_shows_output | Shows last result, not rerun |
| 21 | u_during_flight_after_switching_away | Noop (in_flight), task continues |

---

## Phased Implementation

### Phase 0: Infrastructure Setup (PR-0)

- Create test file: `crates/multitop/tests/upgrade_loop_e2e.rs`
- Verify local tests pass with `cargo test --test upgrade_loop_e2e`
- Confirm `#[ignore]` remote tests compile but don't run

### Phase 1: Core Stream Tests (PR-1)

Implement tests 1-4 and 7-8 (local):

- **Test 1**: Single server basic stream (`ls -l ; ls -l`)
- **Test 2**: Multi-server concurrent (3 servers, `echo` + `ls -l`)
- **Test 3**: Failure exit code (`ls -l ; exit 1`)
- **Test 4**: State machine roundtrip (full App cycle with Msg application)
- **Test 7**: Empty output (`true`)
- **Test 8**: Carriage return cleaning

**Verification:** `cargo test --test upgrade_loop_e2e -- --skip remote --skip test_ui_`

### Phase 2: UI Cycle Tests (PR-2)

Implement tests 11-21 (UI interaction cycle):

- **Test 11**: Upgrade → return → show last result
- **Test 12**: Second `u` in upgrade mode reinitiates
- **Test 13**: Second `u` during flight is no-op
- **Test 14**: Vault locked → password prompt
- **Test 15**: Vault unlocked → password preloaded
- **Test 16**: Modal confirmation flow
- **Test 17**: Switching panes during upgrade preserves task
- **Test 18**: No `upgrade_cmd` → message shown
- **Test 19**: Output persists across view switches
- **Test 20**: Returning to completed shows output (not rerun)
- **Test 21**: `u` during flight after switching away → no-op

**Verification:** `cargo test --test upgrade_loop_e2e -- test_ui_`

### Phase 3: Security & Edge Case Tests (PR-3)

Implement tests 5-6, 9-10, R6, R10:

- **Test 5**: Vault password preloaded
- **Test 6**: Lock prevents concurrent (local lock contention)
- **Test 9**: Generation staleness
- **Test 10**: State persists across App restart
- **R6**: Remote connection failure
- **R10**: Remote agent deployment

**Verification:** `cargo test --test upgrade_loop_e2e -- --skip ui`

### Phase 4: Full Remote Coverage (PR-4)

Implement remote tests R1-R5, R7-R9:

- **R1**: Remote basic command
- **R2**: Remote with sudo password
- **R3**: Remote failure exit code
- **R4**: Remote empty command
- **R5**: Remote lock contention
- **R7**: Remote multiline ordering
- **R8**: Remote stderr capture
- **R9**: Remote large output

**Verification:** `cargo test --test upgrade_loop_remote_e2e -- --ignored`

### Phase 5: CI Gate Integration (PR-5)

- Add `cargo test --test upgrade_loop_e2e` (non-ignored tests) to `ci.yml`
- Add comment template for running remote tests manually
- Verify coverage threshold maintained

**Verification:** `cargo test --workspace`

---

## Implementation Notes

### Key API methods to test against

| Method | File | Purpose |
|--------|------|---------|
| `App::run_upgrade()` | `app.rs:202` | Generates `Command::RunUpgrade` per panel |
| `App::confirm_upgrade()` | `app.rs:254` | Records timestamp, saves state, calls `run_upgrade()` |
| `App::switch_stats()` | `app.rs:169` | Saves upgrade `view` to `last_upgrade`, switches to Monitor |
| `App::show_upgrade_output()` | `app.rs:184` | Restores `last_upgrade` into view, sets mode to Upgrade |
| `App::had_upgrade()` | `app.rs:83` | True if any panel `upgrade_state != NIL` |
| `App::in_upgrade()` | `app.rs:120` | True if any panel `mode == Upgrade` |
| `App::upgrades_in_flight()` | `app.rs:77` | True if any panel `upgrade_state == STARTED` |
| `spawn_upgrade()` | `tasks.rs:143` | SSH/local shell execution, streams Msg to channel |
| `ssh::spawn_command()` | `ssh.rs:280` | Returns `Child` for stdout/stderr streaming |
| `state::save_state()` | `state.rs:44` | Writes `state.toml` with `last_update` |

### Test patterns to follow

1. **Local tests:** Use `spawn_upgrade()` directly (as `e2e_password_upgrade_test.rs` does)
2. **State machine tests:** Use `App::apply(Msg{...})` to simulate message handling (as `app_test.rs` does)
3. **State persistence:** Use `tempfile::TempDir` for config/state files
4. **Mock store:** Call `password_store::enable_mock_store()` / `clear_mock_store()` at test start
5. **Async timeout:** Wrap channel reads with `tokio::time::timeout(Duration::from_secs(5))`

---

## Remote Tests (SSH)

These tests run against real remote machines over SSH. All are `#[ignore]` — run with:
```
cargo test --test upgrade_loop_remote_e2e -- --ignored
```

Requires a reachable SSH host configured via environment variables:
- `MULTITOP_TEST_SSH_HOST` — hostname or IP (default: `127.0.0.1`)
- `MULTITOP_TEST_SSH_USER` — SSH username (default: current user)
- `MULTITOP_TEST_SSH_PORT` — SSH port (default: `22`)

### Test R1: `test_remote_upgrade_basic_command`

- SSH into real host, run `ls -l ; ls -l`
- Full App → run_upgrade → spawn_upgrade → stream cycle

**Assertions:**
- `AuxBegin` received with correct panel/gen
- `AuxDone { success: true }`
- Output lines contain real `ls -l` output (file listings, not error messages)
- Output contains at least 10 lines (real `ls -l` on most systems produces this)

### Test R2: `test_remote_upgrade_with_sudo_password`

- SSH into real host with sudo password provided
- Run `sudo ls -l` as upgrade_cmd
- Password passed to `spawn_upgrade`

**Assertions:**
- Upgrade completes with `success: true`
- No "sudo" error tips in output (password was accepted)
- Output contains file listings from sudo command

### Test R3: `test_remote_upgrade_failure_exit_code`

- SSH into real host, run `ls -l ; exit 42`

**Assertions:**
- `AuxDone { success: false }`
- Output contains file listings (command partially ran)
- No panic or crash in the upgrade task

### Test R4: `test_remote_upgrade_empty_command`

- SSH into real host, run `true` (no output)

**Assertions:**
- `AuxBegin` + `AuxDone { success: true }`
- Zero `AuxLine` messages

### Test R5: `test_remote_upgrade_lock_contention`

- SSH into real host, run a slow command (`sleep 5 && ls -l`)
- Immediately launch second upgrade (`ls -l`)

**Assertions:**
- First upgrade completes with `success: true` or continues running
- Second upgrade either:
  - Gets lock contention error ("Upgrade already in progress"), OR
  - Waits and completes (lock was released by first)

### Test R6: `test_remote_upgrade_connection_failure`

- SSH into unreachable host (e.g., `192.0.2.1` — TEST-NET, RFC 5737)
- Run any command

**Assertions:**
- `Msg::Status` with connection error message
- No panic, no hang (timeout within reasonable bounds)
- Task completes (channel closes)

### Test R7: `test_remote_upgrade_multiline_output_ordering`

- SSH into real host, run:
  ```
  echo STEP_A; sleep 0.1; echo STEP_B; sleep 0.1; echo STEP_C
  ```

**Assertions:**
- Lines received in exact order: `STEP_A`, `STEP_B`, `STEP_C`
- No reordering, no dropped lines

### Test R8: `test_remote_upgrade_stderr_captured`

- SSH into real host, run `echo OUT && echo ERR >&2`

**Assertions:**
- Both stdout (`OUT`) and stderr (`ERR`) lines appear in `AuxLine` messages
- Stderr lines are prefixed or distinguishable

### Test R9: `test_remote_upgrade_large_output`

- SSH into real host, run `seq 1 1000`

**Assertions:**
- At least 1000 `AuxLine` messages received
- Lines are in order (`1` through `1000`)
- No truncation or crash from large output volume

### Test R10: `test_remote_upgrade_agent_deployment`

- SSH into real host where agent binary does NOT exist at expected hash path
- Connect and verify agent is auto-uploaded

**Assertions:**
- Connection succeeds (agent deployed and started)
- Monitor data received (protocol handshake completes)
- Agent binary exists at expected path on remote host

---

## Remote Test Summary

| # | Test Name | Key Check | Requires |
|---|-----------|-----------|----------|
| R1 | remote_upgrade_basic_command | ≥10 real `ls -l` lines | SSH access |
| R2 | remote_upgrade_with_sudo_password | No sudo tips, success | SSH + sudo |
| R3 | remote_upgrade_failure_exit_code | success=false, partial output | SSH access |
| R4 | remote_upgrade_empty_command | Zero AuxLine | SSH access |
| R5 | remote_upgrade_lock_contention | Second gets error or waits | SSH access |
| R6 | remote_upgrade_connection_failure | Status error, no panic | Unreachable host |
| R7 | remote_upgrade_multiline_output_ordering | STEP_A/B/C in order | SSH access |
| R8 | remote_upgrade_stderr_captured | Both stdout+stderr lines | SSH access |
| R9 | remote_upgrade_large_output | ≥1000 lines in order | SSH access |
| R10 | remote_upgrade_agent_deployment | Agent deployed, protocol works | SSH access |

---

## Conventions

- `#[tokio::test]` for async tests
- `enable_mock_store()` / `clear_mock_store()` at start of each local test
- `tempfile::TempDir` for isolation
- No emoji in test output
- Local tests are deterministic (no network, no SSH, no timing dependencies)
- UI cycle tests use `App::apply(Msg{...})` directly — no terminal rendering
- Remote tests use `#[ignore]` and environment variables for host config
- Follow existing test patterns from `app_test.rs` and `e2e_password_upgrade_test.rs`
