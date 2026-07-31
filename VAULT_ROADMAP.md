# Vault Implementation Roadmap

## Phase 0: Foundation (Week 1) ✓
- [x] Password storage in macOS Keychain / Linux Secret Service (existing `password_store`)
- [x] Per-server sudo password override (existing)
- [x] SSO master password (existing)

---

## Phase 1: Local Encrypted Vault File (Week 2) — IMPLEMENTED
**Goal**: Single encrypted file at `~/.local/share/multitop/vault.bin` storing `host:port → password` map.

### 1.1 Crypto Primitives
- [x] AES-256-GCM encryption/decryption (`aes-gcm`)
- [x] Argon2id key derivation (`argon2`) with auto-tuned params (RAM-based)
- [x] Ed25519 signing/verification (`ed25519-dalek`)
- [x] HKDF-SHA256 key separation (`hkdf`)

### 1.2 File Format
- [x] Binary format: `magic | version | salt | argon2_params | wrapped_keys | nonce | ciphertext | ed25519_pk | signature`
- [x] Wrapped key entries: `type:u8 | len:u16 | data:bytes` (Argon2id, SE, TPM2)
- [x] Atomic write (tmp + rename + dir fsync)
- [x] File perms 0600, dir perms 0700

### 1.3 Argon2id Wrapper (Always Present)
- [x] `wrap_argon2id(vault_key, password, salt, params) → wrapped_key`
- [x] `unwrap_argon2id(wrapped_key, password, salt, params) → vault_key`
- [x] Params auto-tuned at creation (RAM/4, clamped 64MiB-1GiB)

### 1.4 Vault API (Sync)
- [x] `Vault::new(config) → Vault`
- [x] `vault.initialize(password) → Result<()>`
- [x] `vault.unlock_biometric(fallback_to_password) → Result<(UnlockedVault, UnlockResult)>`
- [x] `vault.unlock_password(password) → Result<UnlockedVault>`
- [x] `unlocked.get_password(host) → Option<SecretString>`
- [x] `unlocked.set_password(host, password) → Result<()>`
- [x] `unlocked.save() → Result<()>`

### 1.5 Tests
- [x] Round-trip: init → unlock → get/set → save → reload → get
- [x] Wrong password fails
- [x] Tampered file fails signature check
- [ ] Concurrent unlock attempts (not tested)

---

## Phase 2: macOS Secure Enclave (Week 3) — STUB only
**Goal**: Touch ID / Face ID unlock via Secure Enclave EC P-256 key.

### 2.1 Key Management
- [ ] Generate/retrieve persistent EC P-256 key in Secure Enclave (`security-framework`)
- [ ] Access control: `kSecAccessControlDevicePasscode | kSecAccessControlBiometryCurrentSet`
- [ ] Store key handle in vault wrapper

### 2.2 Wrap/Unwrap
- [ ] `wrap(vault_key) → ECIES(ciphertext)` using SE public key (no auth required)
- [ ] `unwrap(wrapped_key) → vault_key` via SE private key (requires Touch ID/Face ID)
- [ ] Handle `errSecAuthFailed` → fallback to password
- [ ] Handle `errSecInvalidKeyRef` (macOS update, Touch ID disabled) → auto re-bind on next password unlock

### 2.3 Integration
- [ ] Auto-create SE wrapper on `initialize()` if available
- [ ] Re-bind SE wrapper on successful password unlock if SE available

**Status**: `secure_enclave.rs` is a **stub** — every method returns `Err(PlatformNotSupported)`. Zero lines of real SE integration.

---

## Phase 3: Linux fprintd + TPM2 (Week 4) — STUB only
**Goal**: Fingerprint unlock on Linux with optional TPM2 sealing.

### 3.1 fprintd (DBus via `zbus`)
- [ ] Connect to `net.reactivated.Fprint` system bus
- [ ] Claim device, `VerifyStart(finger)`, poll `GetStatus` until `verify-match`/`verify-failed`
- [ ] 30s timeout, retry on `verify-retry-scan`
- [ ] Release device on completion

### 3.2 TPM2 (Optional, `tss-esapi`)
- [ ] Create primary key in owner hierarchy
- [ ] Create child key with PCR policy (fprintd auth) + authValue
- [ ] Seal vault key to TPM
- [ ] On verify-match: unseal with TPM

### 3.3 Fallback
- [ ] If no fprintd/TPM2 → Argon2id password only

**Status**: `fprintd.rs` is a **stub** — `FingerprintVerifier::new()` always returns `Err(PlatformNotSupported)`. TPM2 has no implementation at all. No zbus/tss-esapi dependencies in Cargo.toml.

---

## Phase 4: Advanced Features (Week 5) — PARTIAL

### 4.1 Key Rotation / Password Change
- [x] `change_password(old, new) → Result<()>` — implemented in `api.rs`
- [ ] `rotate_key() → Result<()>` — explicit rotation without password change (not separate)
- [ ] Re-wrap vault key with all available wrappers (only Argon2id, SE/TPM2 stubs)

### 4.2 Rollback Protection ⚠️ NOT IMPLEMENTED
- [x] Monotonic counter in signed header (exists)
- [ ] Store last-seen counter in Keychain/TPM
- [ ] Reject vault if counter ≤ stored

### 4.3 Rate Limiting ⚠️ NOT IMPLEMENTED
- [ ] In-memory exponential backoff (1s, 2s, 4s... max 60s)
- [ ] Persist `failed_attempts` + `lockout_until` in vault header
- [ ] Reset on success

### 4.4 Canary
- [x] Random string in decrypted contents (16-byte hex canary)
- [x] Verify on unlock → detect corruption/tampering

---

## Phase 5: UI Integration (Week 6) — PARTIAL

### 5.1 Settings Screen (Press 'e' → Tab: "Passwords")
- [ ] Vault status indicator: locked/unlocked
- [ ] [U] Unlock Vault button
- [ ] [L] Lock Vault button
- [ ] [R] Rotate Key button
- [x] [A] Add / [D] Delete server passwords in vault (via `PasswordAction::Save` + vault set_password when unlocked)

### 5.2 Upgrade Flow — IMPLEMENTED
- [x] On 'u': if vault locked → show TUI password prompt → unlock → get passwords per server → spawn upgrades
- [x] Cache unlocked vault for session (multiple upgrades)
- [ ] Touch ID prompt on first unlock (skipped — goes straight to password prompt)

---

## Phase 6: Hardening & Polish (Week 7) — NOT STARTED
- [ ] `mlock` vault key on Linux (best effort)
- [ ] Secure delete on key rotation (`shred` / `srm`)
- [ ] Migration: import existing per-server keychain passwords
- [ ] CI: test vault crate in isolation
- [ ] Fuzz: vault file parser (`cargo-fuzz`)

---

## SECURITY GAPS (From 3 Rounds of Review + Code Audit)

### OPEN: P0 — No Rate Limiting on Password Attempts
**Risk**: Any local process (or attacker with the vault file) can try passwords instantly. No backoff, no lockout. A weak sudo password can be brute-forced in minutes.
**File**: `api.rs:unlock_with_password()` — unbounded.
**Fix**: Add in-memory exponential backoff (3 failures → 1s, 4→2s, 5→4s... max 60s). Persist count in header to survive process restart.
**Source**: Security Review R2 §P1-7, R2 table P2.

### OPEN: P0 — No Rollback Protection (Stored Counter)
**Risk**: Attacker copies vault file, modifies passwords, replaces with old version. Counter in header exists but is never compared against a stored value — rollback is undetectable.
**File**: `header.counter` exists but last-seen counter is never stored or verified.
**Fix**: Store last_seen_counter in Keychain (`keyring` crate, same service as passwords). Reject vault if counter ≤ stored.
**Source**: Security Review R1 §5, Fixed Plan §Rollback Protection.

### OPEN: P1 — No HKDF Key Separation (Same Key for AES-GCM + Ed25519)
**Risk**: `VaultKey.0` is used directly as AES-256-GCM key (`crypto.rs:encrypt_vault`, line 201: `Aes256Gcm::new(&GenericArray::clone_from_slice(key.as_bytes()))`) AND as the HKDF input for Ed25519 signing key derivation (`derive_signing_key`, line 39). NIST SP 800-108 requires distinct keys for distinct algorithms.
**Impact**: Theoretical cross-protocol attack surface between AES-GCM and Ed25519.
**Fix**: Derive sub-keys via HKDF-Expand with distinct labels:
  - `enc_key = HKDF-Expand(vault_key, b"aes-gcm-enc")`
  - `sig_key = HKDF-Expand(vault_key, b"ed25519-sig")`
  - `vault_key` itself should be the output of the unwrap, not used directly.
**Source**: Security Review R3 §P2-1.

### OPEN: P1 — macOS Memory Detection Uses Total RAM, Not Available
**Risk**: `get_available_memory_kib()` on macOS reads `hw.memsize` (total physical RAM), not available memory. On a machine with 256 GiB total but high memory pressure, Argon2id could select parameters that cause OOM or swap.
**File**: `crypto.rs:get_available_memory_kib()` line 141-144.
**Fix**: Use `vm_statistics`/`host_statistics` to get page-free count, or use `sysctl vm.page_free_count`.

### OPEN: P1 — Secure Enclave is a STUB
**Risk**: All three reviews assume Touch ID unlock works. It doesn't — the entire `secure_enclave.rs` module is a stub that always returns unavailable (13 lines of actual code, all error returns).
**Impact**: No biometric unlock path exists. All unlocks use password only.
**Fix**: Implement `security-framework` Secure Enclave wrap/unwrap. Add `security-framework` dependency.
**Source**: Security Review R1 §6, R2 §P2-5, R3 §P2-3.

### OPEN: P1 — fprintd is a STUB
**Risk**: Linux biometric path similarly stubbed out (20 lines, all errors). No zbus dependency.
**Impact**: Linux users get Argon2id password only.
**Fix**: Implement fprintd via `zbus` D-Bus.
**Source**: Security Review R1 §13, R2 §P2-6.

### OPEN: P1 — No Vault Migration Framework
**Risk**: Vault format is versioned (version byte) but there is zero migration logic. If format changes (add TPM2 wrapper type, change Argon2id encoding), existing vaults become permanently unreadable.
**File**: `format.rs` has version check but no `migrate()`.
**Source**: Security Review R2 §P1-4.

### OPEN: P2 — Re-bind Race Condition (No File Locking)
**Risk**: `save()` in `api.rs` writes via atomic rename but has no file locking. Two concurrent processes (two multitop instances) both re-binding biometric wrapper after password unlock can race → lost wrapper.
**Impact**: Intermittent biometric failure next unlock (if SE existed). For now: no SE → low impact, but will matter when SE is implemented.
**Fix**: Add `flock()` or `fcntl` advisory lock around save.
**Source**: Security Review R2 §P1-1.

### OPEN: P2 — `sign_vault` Gets Ciphertext, Should Sign (Header || Ciphertext)
**Risk**: `api.rs:save()` line 74: `self.header.signature = crypto::sign_vault(&self.vault_key, &self.header.signed_data(&ciphertext))`. Verified. The `signed_data()` method includes header fields + ciphertext. **This is correct** — no issue here on closer inspection.
**Status**: No issue.

### OPEN: P2 — No Secure Deletion on Key Rotation
**Risk**: `change_password()` writes a new vault file but the old file data may persist on SSD (wear leveling, journal).
**Impact**: An attacker with forensic access could recover old vault file and attempt offline attack.
**Fix**: Best-effort `shred` or `srm` before rename, or document that FileVault/LUKS renders this moot.
**Source**: Security Review R3 §P3-5.

### OPEN: P3 — No macOS `mlock` Support
**Risk**: Vault key and decrypted passwords live in normal process memory. If the process is swapped, coredumped, or `/proc/pid/mem` read, keys are exposed.
**Fix**: Use `libc::mlock` on the vault key buffer (best-effort, no failure on EPERM).
**Source**: Security Review R1 §9.

---

## Summary: Security Gap Severity

| Severity | Count | Key Items |
|----------|-------|-----------|
| **P0** | 2 | Rate limiting, rollback protection |
| **P1** | 5 | HKDF key separation, macOS memory detection, SE stub, fprintd stub, migration framework |
| **P2** | 2 | Re-bind race, secure deletion |
| **P3** | 1 | mlock |

**Biometric unlock (Touch ID / fingerprint) is entirely unimplemented** — both macOS SE and Linux fprintd modules are stubs. The vault is password-only in practice.

---

## Acceptance Criteria per Phase

| Phase | Must Work | Status |
|-------|-----------|--------|
| 1 | `vault.initialize("pass")` → `vault.unlock_password("pass")` → get/set passwords → save → reload | ✓ |
| 2 | On Mac with Touch ID: `unlock_biometric(true)` → Touch ID prompt → success without password | ✗ (stub) |
| 3 | On Linux with fingerprint reader: `unlock_biometric(true)` → fingerprint prompt → success | ✗ (stub) |
| 4 | Wrong password 5x → 32s lockout; vault file replaced with old copy → rejected | ✗ (not implemented) |
| 5 | 'e' → Vault tab shows status; 'u' uses vault passwords automatically | Partial (upgrade flow ✓, settings ✗) |
| 6 | `cargo fuzz` runs 1M iterations without crash; all tests pass in CI | ✗ (not started) |

---

## Dependencies Summary (Actual)

```toml
# Current vault Cargo.toml (cf. planned)
aes-gcm = "0.10"       ✓
argon2 = "0.5"         ✓
ed25519-dalek = "2"    ✓
hkdf = "0.13"          ✓
rand = "0.8"           ✓
secrecy = "0.10"       ✓
sha2 = "0.10"          ✓
zeroize = "1.7"        ✓
serde = "1.0"          ✓
serde_json = "1.0"     ✓
dirs = "5"             ✓
thiserror = "1.0"      ✓
tokio = "1"            ✓
hex = "0.4"            ✓ (not in plan)
rpassword = "7.3"      ✓ (not in plan)

# Missing from current Cargo.toml:
security-framework =    ✗ (macOS SE needs this)
security-framework-sys = ✗
zbus =                  ✗ (Linux fprintd needs this)
zvariant =              ✗
tss-esapi =             ✗ (TPM2, optional)
serde_bytes =           ✗ (in Cargo.lock transitively but not direct dep)
```

---

## Notes

- **No post-quantum KEM** in MVP — AES-256-GCM + Ed25519 is sufficient for local file threat model
- **TPM2 is optional** — deferred until fprintd+password primary path is stable
- **Key separation is NOT implemented** — `VaultKey` is used directly in AES-GCM and as HKDF input for Ed25519
- **Vault is device-local** — no sync, no cloud. Each machine has its own vault.
- **Biometric (SE/fprintd) is entirely unimplemented** — both modules are stubs returning `PlatformNotSupported`
- **Multitop integration**: upgrade flow (Phase 5.2) is complete; settings UI (Phase 5.1) is not
