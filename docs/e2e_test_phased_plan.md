# E2E Test Implementation Plan — Phased (Corrected)

**Scope:** Fill **actual gaps** from `docs/e2e_test_gaps.md` after auditing existing tests  
**Existing Tests:** ~130 tests across 13 test files + inline unit tests  
**Real Gaps:** ~85 new tests needed (not ~143 — removed duplicates)

---

## Coverage Audit Summary

| Module | Existing Tests | Status |
|--------|----------------|--------|
| `ansi.rs` | 18 inline | ✓ Complete |
| `sparkline.rs` | 1 inline | ✓ Complete |
| `state.rs` | 2 inline | ✓ Complete |
| `password_store.rs` | 1 inline (mock) | ✓ Complete |
| `fetch_render.rs` | 14 in `fetch_render_test.rs` | ✓ Complete |
| `app.rs` | 26 in `app_test.rs` | ✓ Complete |
| `ui.rs` (layout) | 14 in `ui_test.rs` | ✓ Complete |
| `passwords.rs` | 5 in `server_settings_test.rs` | ⚠ Partial |
| `password_actions.rs` | 4 in `server_settings_test.rs` + `e2e_password_upgrade_test.rs` | ⚠ Partial |
| `ssh.rs` | 2 in `app_test.rs` (`is_local`, `cleanup_old_agents`) | ✗ Major gaps |
| `config.rs` | 1 in `app_test.rs` (`save_servers`) | ✗ Major gaps |
| `tasks.rs` | `spawn_upgrade` in E2E tests | ✗ Missing `spawn_fetch`, `spawn_docker`, `spawn_monitor` |
| `panel.rs` | 0 | ✗ No tests |
| `modals.rs` | 0 | ✗ No tests |
| `config_ui.rs` | 0 | ✗ No tests |
| `ssh_opts.rs` | 0 | ✗ No tests |
| `fmt.rs` | 1 in `app_test.rs` | ✗ Major gaps |
| `render_payload.rs` | 0 | ✗ No tests |
| `refit.rs` | 0 | ✗ No tests |
| `vault.rs` | 7 in `vault_upgrade_e2e.rs` | ✓ Complete |
| `upgrade_loop` | 21 in `upgrade_loop_e2e.rs` + 10 ignored in `upgrade_loop_remote_e2e.rs` | ✓ Complete |

---

## Phase 1: Pure Function Unit Tests (Inline `#[cfg(test)]`)
**Target:** 7 modules, ~35 tests | **Duration:** 1 session | **Risk:** None

### 1.1 `refit.rs` — 6 tests
- [ ] `refit_header_wide_terminal` — full header with padding
- [ ] `refit_header_narrow_terminal` — truncation at `target_cols <= disp_w`
- [ ] `refit_header_fullwidth_chars` — Unicode width (0xFF01..0xFF5E = 2 cols)
- [ ] `refit_line_horizontal_rule` — `────` line becomes spaced rule
- [ ] `refit_line_plain_text` — plain text passes through unchanged
- [ ] `refit_line_empty` — empty string returns empty

### 1.2 `ssh_opts.rs` — 8 tests
- [ ] `arch_from_uname_x86_64` / `aarch64` / `armv7` / `unknown`
- [ ] `arch_binary_selection` — each arch returns correct embedded bytes
- [ ] `arch_hash_label_word_consistency` — cross-reference all fields
- [ ] `sh_quote_empty` / `sh_quote_simple` / `sh_quote_single_quotes` / `sh_quote_idempotent`

### 1.3 `fmt.rs` — 6 tests
- [ ] `error_line_prefix` — red bold "✗ "
- [ ] `status_line_prefix` — cyan "→ "
- [ ] `header_line_prefix` — bold cyan
- [ ] `unixtime_to_str_recent` / `old` / `zero` — formatting consistency
- [ ] `error_line_contains_input` — input preserved
- [ ] `status_line_contains_input` — input preserved

### 1.4 `render_payload.rs` — 3 tests
- [ ] `dispatch_monitor` — forwards to `multitop_agent::render`
- [ ] `dispatch_docker` — forwards to `multitop_agent::docker::render`
- [ ] `dispatch_fetch` — forwards to `crate::fetch_render::render_fetch`

### 1.5 `fetch_render.rs` (internal helpers) — 3 tests
- [ ] `center_header_widths` — centered at various cols
- [ ] `pick_lines_crop_center` — logo cropping logic
- [ ] `load_db_non_empty` — embedded logo DB loads

### 1.5 `ssh.rs` (pure parsers) — 5 tests
- [ ] `parse_need_agent_detects` — "NEED_AGENT: <hash>" extraction
- [ ] `parse_need_agent_ignores_other_lines` — no false positives
- [ ] `wrap_with_upgrade_lock_contains_lock` — flock logic present
- [ ] `wrap_with_local_upgrade_lock_contains_lock` — flock logic present
- [ ] `cleanup_old_agents_command_structure` — well-formed shell (idempotent with app_test)

### 1.6 `config.rs` (pure validators) — 4 tests
- [ ] `validate_host_valid` / `empty` / `too_long` / `unicode` / `special_chars`
- [ ] `validate_user_valid` / `empty` / `too_long` / `special_chars`
- [ ] `server_target_with_user_port` / `without_user` / `default_port`
- [ ] `parse_valid_toml_structure` — servers, theme, sparklines parsed

---

## Phase 2: Config & SSH Parsing Integration Tests (New file)
**Target:** 2 modules, ~15 tests | **Duration:** 1 session | **Risk:** Low

### 2.1 `crates/multitop/tests/config_e2e_test.rs` — 10 tests
- [ ] `test_config_load_valid_toml` — parses servers, theme, sparklines
- [ ] `test_config_load_missing_file` — returns default config
- [ ] `test_config_load_malformed_toml` — returns `ConfigError`
- [ ] `test_config_save_servers_roundtrip` — write → read equals original
- [ ] `test_config_save_theme_show_sparklines` — config file updated
- [ ] `test_ssh_config_parse_multiple_hosts` — Host alias expansion
- [ ] `test_ssh_config_parse_wildcards` — `Host *` fallback
- [ ] `test_ssh_config_parse_real_file` — ~/.ssh/config fixture
- [ ] `test_config_path_precedence` — --config > env > default > legacy
- [ ] `test_server_deduplication` — same host:port merged (extend app_test)

### 2.2 `crates/multitop/tests/ssh_e2e_test.rs` — 5 tests (local-only)
- [ ] `test_local_spawn_command_no_password` — `echo hello` → stdout
- [ ] `test_local_spawn_command_with_mock_password` — sudo -S with mock store
- [ ] `test_local_spawn_command_upgrade_lock_prevents_concurrent` — 2nd blocks
- [ ] `test_local_spawn_agent_finds_binary` — current_exe or multitop-agent
- [ ] `test_wrap_lock_idempotent` — double-wrap = single lock

---

## Phase 3: Password UI & Actions (Extend existing + inline)
**Target:** 2 modules, ~15 tests | **Duration:** 1 session | **Risk:** Medium

### 3.1 `passwords.rs` (inline unit tests) — 8 tests
- [ ] `server_key_draft_field_navigation` — Tab/Up/Down cycles 0..4
- [ ] `server_key_draft_char_input` — chars append to active field
- [ ] `server_key_draft_backspace` — pops from active field
- [ ] `server_key_draft_enter_valid` — returns `ApplyServers` or `SaveServerWithPassword`
- [ ] `server_key_draft_enter_invalid` — sets notice, keeps draft
- [ ] `server_key_draft_esc_cancels` — draft = None
- [ ] `password_key_sparkline_toggle` — returns `ToggleSparklines`
- [ ] `password_key_delete_server_last_one_shows_notice` — len=1 edge case

### 3.2 `password_actions.rs` (extend `server_settings_test.rs`) — 7 tests
- [ ] `test_save_server_with_password` — combined server + password save
- [ ] `test_delete_password_removes_from_keychain` — Delete → keychain cleanup
- [ ] `test_save_sso_propagates_to_all_panels` — SSO password → all panels
- [ ] `test_delete_sso_clears_all` — DeleteSso → all panels cleared
- [ ] `test_toggle_sparklines_persists_config` — config file updated
- [ ] `test_save_resume_upgrade_false` — resume_upgrade=false doesn't trigger upgrade
- [ ] `test_apply_servers_preserves_existing_passwords` — old panel passwords kept

---

## Phase 4: Core Runtime — Tasks & Panels (New files)
**Target:** 2 modules, ~18 tests | **Duration:** 2 sessions | **Risk:** High

### 4.1 `crates/multitop/tests/tasks_e2e_test.rs` — 8 tests
- [ ] `test_spawn_fetch_creates_task` — panel.mode=Fetch, aux=false
- [ ] `test_spawn_docker_creates_task` — panel.mode=Docker, aux=false
- [ ] `test_spawn_monitor_creates_task` — panel.mode=Monitor, aux=false
- [ ] `test_spawn_upgrade_generation_tracking` — gen bumped, aux flag set (extend existing)
- [ ] `test_spawn_upgrade_sets_mode_and_state` — panel.mode=Upgrade, state=STARTED
- [ ] `test_spawn_upgrade_saves_state_file` — upgrade_started_at persisted
- [ ] `test_task_cancellation_on_panel_switch` — old handle aborted
- [ ] `test_concurrent_upgrade_generations_isolated` — gen mismatch drops frame

### 4.2 `crates/multitop/tests/panel_e2e_test.rs` — 10 tests
- [ ] `test_panel_new_initializes_state` — mode=Monitor, upgrade_state=NIL
- [ ] `test_ensure_sudo_password_loads_keychain` — mock store → password set
- [ ] `test_ensure_sudo_password_loads_vault` — vault unlocked → password set
- [ ] `test_ensure_sudo_password_none` — no source → None
- [ ] `test_set_sudo_password_session_only` — from_vault=false → session only
- [ ] `test_set_sudo_password_from_vault` — from_vault=true → password_saved=true
- [ ] `test_show_last_frame_restores_view` — view = last_frame after upgrade
- [ ] `test_password_saved_flag_sync` — keychain save → flag=true, fail → flag=false
- [ ] `test_panel_mode_transitions` — Monitor→Docker→Fetch→Upgrade→Monitor
- [ ] `test_panel_generation_bump_on_mode_change` — gen increments

---

## Phase 5: UI Rendering & Modals (New files)
**Target:** 3 modules, ~15 tests | **Duration:** 1 session | **Risk:** Medium

### 5.1 `crates/multitop/tests/modals_e2e_test.rs` — 6 tests
- [ ] `test_upgrade_modal_content_normal` — last update shown, no warning
- [ ] `test_upgrade_modal_content_interrupted` — red warning shown
- [ ] `test_upgrade_modal_keys` — U/Enter confirm, Esc cancel
- [ ] `test_vault_password_prompt_dots` — input length = dot count
- [ ] `test_vault_password_prompt_error` — error line displayed in red
- [ ] `test_modal_layout_at_min_size` — 20x10 terminal renders without panic

### 5.2 `crates/multitop/tests/config_ui_e2e_test.rs` — 5 tests
- [ ] `test_servers_section_renders_list` — all panels with host:port, user, upgrade_cmd
- [ ] `test_passwords_section_renders_status` — lock icon for saved, circle for unset
- [ ] `test_draft_edit_renders_field_highlight` — `>` on active field
- [ ] `test_notice_renders_yellow` — error/success notices visible
- [ ] `test_theme_colors_applied` — accent, border, bg from palette

### 5.3 `crates/multitop/tests/ui_render_e2e_test.rs` (extend `ui_test.rs`) — 4 tests
- [ ] `test_multi_panel_horizontal_layout` — 3 panels side by side
- [ ] `test_panel_mode_rendering` — Monitor/Docker/Fetch/Upgrade each render
- [ ] `test_keybar_context_aware` — hints change per mode
- [ ] `test_sparkline_rendering_enabled_disabled` — bars vs empty

---

## Phase 6: Upgrade Loop Extensions (Extend `upgrade_loop_e2e.rs`)
**Target:** ~8 tests | **Duration:** 1 session | **Risk:** Low

### 6.1 Extend `crates/multitop/tests/upgrade_loop_e2e.rs` — 8 tests
- [ ] `test_upgrade_vault_fallback_to_keychain` — vault locked → keychain used (extend vault test)
- [ ] `test_upgrade_vault_priority_over_keychain` — vault unlocked → vault used (extend vault test)
- [ ] `test_upgrade_modal_interrupted_warning` — upgrade_started_at > last_update
- [ ] `test_upgrade_carriage_return_cleaned` — \r stripped from stream (partially in app_test)
- [ ] `test_upgrade_empty_output_handled` — zero-byte output → "completed"
- [ ] `test_upgrade_stderr_captured_in_output` — stderr merged to stdout
- [ ] `test_upgrade_multi_server_all_stream` — 3 servers, all output captured
- [ ] `test_upgrade_state_persists_across_app_restart` — state.toml roundtrip

---

## Phase 7: Property Tests & CI Hardening
**Target:** Cross-cutting | **Duration:** 1 session | **Risk:** Low

### 7.1 Property Tests (proptest - new dev dep)
- [ ] `ssh_opts_sh_quote_idempotent` — `sh_quote(sh_quote(x)) == sh_quote(x)`
- [ ] `refit_refit_idempotent` — `refit_line(refit_line(x, w), w) == refit_line(x, w)`
- [ ] `config_parse_load_roundtrip` — `parse(load(x)) == x` for valid configs
- [ ] `ansi_roundtrip_visible_chars` — `plain(line_to_spans(x))` strips only ANSI

### 7.2 CI Integration (`.github/workflows/ci.yml`)
```yaml
- name: Unit tests (lib)
  run: cargo test --package multitop --lib --all-targets

- name: Integration tests (local)
  run: |
    cargo test --package multitop \
      --test upgrade_loop_e2e \
      --test vault_upgrade_e2e \
      --test ssh_e2e_test \
      --test config_e2e_test \
      --test tasks_e2e_test \
      --test panel_e2e_test \
      --test modals_e2e_test \
      --test config_ui_e2e_test \
      --test ui_render_e2e_test \
      --test server_settings_test \
      --test e2e_password_upgrade_test \
      --test fetch_render_test \
      --test ui_test \
      --test app_test

- name: Integration tests (remote)
  if: env.MULTITOP_TEST_SSH_HOST != ''
  run: cargo test --package multitop --test upgrade_loop_remote_e2e -- --test-threads=1

- name: Property tests
  run: cargo test --package multitop --features proptest --lib proptest::
```

### 7.3 Coverage Gate
- [ ] Add `cargo llvm-cov --package multitop --lcov --output-path lcov.info`
- [ ] Fail if line coverage < 80% on `crates/multitop/src/**/*.rs` (exclude `main.rs`, tests)

---

## Test Count Summary (Corrected)

| Phase | Tests | Files | Type |
|-------|-------|-------|------|
| 1: Pure functions | 35 | 7 inline | Unit |
| 2: Config/SSH parsing | 15 | 2 new | Integration |
| 3: Password UI/Actions | 15 | 2 inline + 1 extend | Unit + Integration |
| 4: Tasks & Panels | 18 | 2 new | Integration |
| 5: UI/Modals | 15 | 3 new (1 extend) | Integration |
| 6: Upgrade loop extensions | 8 | 1 extend | Integration |
| 7: Property/CI | ~8 | 1 config + inline | Property |
| **Total** | **~114** | **6 new test files** | |

**vs Original Plan:** -29 tests (removed duplicates), -1 new file

---

## Dependency Graph (Corrected)

```
Phase 1 (pure inline) ──────────────────┐
Phase 2 (config/ssh parsing) ───────────┤
Phase 3 (password UI/actions inline) ───┼──► Phase 4 (tasks/panels)
Phase 4 produces test fixtures ─────────┘       │
                                                ▼
Phase 5 (UI/modals) ◄──────────────────────── Phase 4 fixtures
                                                │
Phase 6 (upgrade extensions) ◄────────────────┘
                                                │
Phase 7 (property/CI) ◄────────────────────────┘
```

---

## Acceptance Criteria

- [ ] All 114 new tests pass locally
- [ ] All existing + new tests pass in CI (local + remote)
- [ ] Line coverage ≥ 80% on `crates/multitop/src/**/*.rs`
- [ ] No `#[ignore]` tests except `upgrade_loop_remote_e2e.rs`
- [ ] Property tests run in CI
- [ ] Gap analysis doc updated with ✓/✗ status per module

---

## Quick Wins (Do First)

These require **no new files** and unblock later phases:

1. **Inline unit tests** (Phase 1) — 35 tests in 7 modules, zero dependencies
2. **Extend `server_settings_test.rs`** — 7 password_actions tests
3. **Extend `upgrade_loop_e2e.rs`** — 8 upgrade loop tests
4. **Inline `passwords.rs` tests** — 8 state machine tests

These 58 tests can be done in 1-2 sessions before creating any new test files.