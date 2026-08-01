# Clippy Fix Plan: Proper Fixes for Overridden Lints

## Status
- [ ] 1. struct_excessive_bools (App struct)
- [ ] 2. too_many_lines (10 functions)
- [ ] 3. option_option (SSO_CACHE)
- [ ] 4. cast_* lints
- [ ] 5. expect_used / unwrap_used
- [ ] 6. pub_underscore_fields
- [ ] 7. missing_panics_doc / missing_errors_doc

---

## 1. struct_excessive_bools (App struct - 14 bools)

**File:** `crates/multitop/src/app.rs`

**Current:** 14 bool fields in `App` struct

**Plan:** Group into state enums

```rust
// App mode / UI state
enum AppMode {
    Running,
    Filtering,
    ShowUpgradeModal,
    ShowSparklines(bool), // or separate flag
}

// Vault state
enum VaultState {
    Locked,
    Unlocked { awaiting_biometric: bool },
    PasswordPrompt { error: Option<String> },
}
```

**Files to update:** `app.rs`, all call sites in `run.rs`, `ui.rs`, `password_actions.rs`, `tasks.rs`

---

## 2. too_many_lines (10 functions)

### 2.1 `App::apply` (156 lines) → `crates/multitop/src/app.rs`
Extract: `handle_monitor_payload`, `handle_docker_payload`, `handle_fetch_payload`

### 2.2 `password_actions::apply` (180 lines) → `crates/multitop/src/password_actions.rs`
Extract: `apply_servers`, `apply_password`, `apply_vault_password`, `apply_upgrade`

### 2.3 `run::event_loop` (159 lines) → `crates/multitop/src/run.rs`
Extract: `init_app`, `spawn_agents`, `main_loop`, `handle_resize`

### 2.4 `run::handle_key` (159 lines) → `crates/multitop/src/run.rs`
Extract: `handle_global_key`, `handle_panel_key`, `handle_config_key`, `handle_password_key`

### 2.5 `ui::keybar_line` (122 lines) → `crates/multitop/src/ui.rs`
Extract: `build_left_spans`, `build_badge_spans`, `build_padding`

### 2.6 `ui::draw` (116 lines) → `crates/multitop/src/ui.rs`
Extract: `draw_panels`, `draw_keybar`, `calculate_layout`

### 2.7 `tasks::spawn_upgrade` (127 lines) → `crates/multitop/src/tasks.rs`
Extract: `build_upgrade_command`, `stream_upgrade_output`

### 2.8 `config_ui::draw` (174 lines) → `crates/multitop/src/config_ui.rs`
Extract: `draw_server_list`, `draw_server_form`, `draw_notice`

### 2.9 `config::load` (108 lines) → `crates/multitop/src/config.rs`
Extract: `load_file`, `parse_toml`, `validate_config`

### 2.10 `config::parse` (108 lines) → `crates/multitop/src/config.rs`
Extract: `parse_servers`, `parse_theme`, `parse_settings`

---

## 3. option_option (SSO_CACHE) → `crates/multitop/src/password_store.rs`

**Current:** `RwLock<Option<Option<String>>>`

**Plan:**
```rust
enum SsoCacheState {
    Uninitialized,
    Loaded(Option<String>), // None = explicitly deleted
}

static SSO_CACHE: RwLock<SsoCacheState> = RwLock::new(SsoCacheState::Uninitialized);
```

Update: `clear_sso_cache`, `load_sso`, `save_sso`, `delete_sso`

---

## 4. cast_* lints

### 4.1 `app.rs` - percentage calculations
Replace `as f32` with checked math:
```rust
let mem_pct = if snap.mem.total > 0 {
    ((snap.mem.used as f64 / snap.mem.total as f64) * 100.0) as f32
} else { 0.0 };
```

### 4.2 `config.rs` - port/history parsing
Use `try_from` with proper error handling (already partially done)

### 4.3 `sparkline.rs` - index calculation
```rust
let idx = ((v / 100.0) * 7.0).round() as usize; // v in [0,100], idx in [0,7]
// Add debug_assert!(idx <= 7);
```

### 4.4 `ui.rs` - panel counts
Use `u32::try_from(panels).expect("panel count fits in u32")` - already done

### 4.5 `state.rs` - timestamp storage
```rust
i64::try_from(v).expect("timestamp fits in i64") // u64 ms since epoch < i64::MAX until year 292278994
```

---

## 5. expect_used / unwrap_used

**Approach per site:**
- Config parsing: return `Result` with `ConfigError`
- `TryFrom` on bounded values: keep `expect` with descriptive message
- Truly impossible cases: add `// SAFETY:` comment + `unwrap()`

**Files:** `config.rs`, `state.rs`, `ui.rs`, `stream.rs`, `tasks.rs`

---

## 6. pub_underscore_fields → `crates/multitop/src/stream.rs`

**Current:** `pub _child: Child`

**Fix:** Rename to `child` and make private, or keep public without underscore.

---

## 7. missing_panics_doc / missing_errors_doc

**Approach:** Add `# Panics` and `# Errors` sections to all public functions.

**Audit command:** `cargo doc --document-private-items 2>&1 | grep -i "missing"`

---

## Implementation Order

1. struct_excessive_bools (highest impact - eliminates state bugs)
2. too_many_lines (enables testability)
3. option_option (type safety)
4. cast_* (audit for precision)
5. expect_used (error handling)
6. pub_underscore_fields (trivial)
7. missing_*_doc (hygiene)