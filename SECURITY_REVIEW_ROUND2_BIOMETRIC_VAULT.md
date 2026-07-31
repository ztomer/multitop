# Security Review Round 2: Fixed Pragmatic Biometric Vault Plan

**Reviewer**: Security Expert Persona
**Date**: 2026-07-30
**Verdict**: **CONDITIONAL PASS** — Significant improvement, but 4 P1 issues and 3 P2 issues remain

---

## Executive Summary

The fixed plan addresses all P0 findings from Round 1. The independent key wrapping architecture correctly eliminates the downgrade attack. However, **four P1 issues** and **three P2 issues** remain that should be fixed before implementation.

---

## P1 Issues (Should Fix Before Implementation)

### 1. Opportunistic Re-Bind Creates Race Condition

**Issue**: In the unlock flow, after successful password unlock, the code attempts to add a biometric wrapper:

```rust
if methods.contains(&0x01) && cfg!(target_os = "macos") {
    let wrapped = wrap_secure_enclave(&self.vault_key)?;
    self.header.add_wrapping(0x01, wrapped);
    self.save()?;  // <-- RACE: vault file rewritten
}
```

**Attack**: If two upgrade processes run concurrently on same machine:
1. Process A unlocks with password, starts re-bind
2. Process B unlocks with password, starts re-bind
3. Both write vault file → one wins, one loses
4. Loser's biometric wrapper discarded → next unlock falls back to password

**Impact**: Intermittent biometric failure, not a security breach. But creates confusing UX.

**Fix**: 
- Use file locking (`flock`/`fcntl`) during `save()`
- Or: Defer re-bind to background task with mutex
- Or: Only re-bind on explicit "Save biometric" user action in settings

### 2. Ed25519 Signing Key Management Unspecified

**Issue**: Plan mentions `sign_vault(&signing_key, ...)` but doesn't specify:
- Where does `signing_key` come from?
- How is it stored?
- What happens if it's rotated?

**Current implied design**: Static Ed25519 keypair embedded in binary or generated on first run.

**Problems**:
- Embedded in binary → same key for all users → no tamper detection value
- Generated on first run → stored where? If in vault file, circular dependency
- If in Keychain/TPM → adds complexity

**Fix**: 
- **Option A**: Derive signing key from `vault_key` (HKDF) — no separate key needed
- **Option B**: Use AES-GCM tag as integrity (already authenticates ciphertext) — drop Ed25519
- **Option C**: Store Ed25519 keypair in Keychain/TPM alongside rollback counter

**Recommendation**: Option A — `signing_key = HKDF(vault_key, "signing")`. Simpler, no extra storage.

### 3. Argon2id Parameters Not Configurable Per-User

**Issue**: Hardcoded `t=10, m=256MiB, p=4` assumes modern hardware. On:
- Older Mac (2015): 256 MiB may cause OOM / swap → 10+ second delay
- Raspberry Pi / embedded: 256 MiB may exceed available RAM
- CI / containers: Memory limits may kill process

**Impact**: Vault unlock fails or is unusably slow on constrained devices.

**Fix**: 
- Store parameters in vault header (`t, m, p` fields)
- On creation: auto-detect available memory, choose safe defaults
- Allow user override in config.toml
- Minimum: `t=3, m=64MiB` (with warning), maximum: `t=20, m=1GiB`

### 4. No Vault Version Migration Path

**Issue**: Vault format is versioned (`Version: 1`), but no migration logic specified. When format changes (e.g., adding TPM2 support, changing Argon2id params), existing vaults become unreadable.

**Impact**: Users lose all stored passwords on upgrade → must re-enter manually.

**Fix**:
- Include `migration_version` in header
- Implement `migrate(old_vault, new_version)` function
- Test: v1 → v2 migration in CI
- Document: "Vault format changes require manual re-entry" if migration not implemented

---

## P2 Issues (Should Fix Soon After)

### 5. Secure Enclave Key Persistence Across macOS Updates

**Issue**: Secure Enclave keys can be invalidated by:
- macOS major version upgrade (sometimes)
- Secure Enclave firmware update
- User disabling/re-enabling Touch ID
- Hardware repair (logic board replacement)

**Impact**: Vault becomes unlockable only via password. Biometric wrapper becomes useless but remains in file.

**Fix**:
- On unwrap failure with `errSecAuthFailed` / `errSecInvalidKey`: silently remove that wrapper, fall back to password
- On successful password unlock, re-create biometric wrapper (current re-bind logic handles this)
- Document: "Biometric unlock may stop working after macOS updates — use password once to restore"

### 6. TPM2 Authorization Policy Too Simplistic

**Issue**: Plan says "TPM2 key with policy: PCR 16 (fprintd auth) + authValue". But:
- PCR 16 is not standard for fprintd (varies by distro)
- fprintd doesn't reliably extend PCRs
- TPM2 policy should be: `PolicyAuthorize` with fprintd as external authorizer, OR `PolicyPCR` for specific boot state + `PolicyPassword` for user auth

**Impact**: TPM2 wrapper may not work on many Linux configurations.

**Fix**:
- Implement TPM2 wrapper as **optional**, behind feature flag
- Primary Linux path: `fprintd verify` → then Argon2id(password) (no TPM)
- TPM2: Advanced opt-in with documented kernel/fprintd requirements
- Don't block vault on TPM2 availability

### 7. No Rate Limiting on Password Unlock Attempts

**Issue**: `prompt_password()` can be called repeatedly. Attacker with local access can:
- Cancel biometric prompt → password prompt
- Try passwords rapidly (no delay, no lockout)
- Offline attack not needed — online brute force

**Impact**: Weak sudo passwords vulnerable to local brute force.

**Fix**:
- Track failed attempts in memory (reset on successful unlock)
- After 3 failures: exponential backoff (1s, 2s, 4s, 8s... max 60s)
- After 10 failures: require biometric (if available) or 5-minute lockout
- Persist attempt count in vault header (survives process restart)

---

## P3 / Informational

### 8. JSON Schema Validation Missing

**Issue**: `decrypt_vault` → `serde_json::from_slice` without schema validation. Malformed JSON could cause panic or logic errors.

**Fix**: Use `serde_json::from_slice` with `deny_unknown_fields`, validate required fields.

### 9. No Vault Integrity Check on Load (Before Decrypt)

**Issue**: Plan verifies signature after decrypt. Should verify signature **before** decrypt to fail fast on tampering.

**Fix**: Reorder: `verify_signature(header, ciphertext, sig)` → then `decrypt()`.

### 10. `mlock` Not Portable

**Issue**: `mlock` requires `CAP_IPC_LOCK` on Linux, may fail in containers. macOS has different API.

**Fix**: Make `mlock` best-effort, log warning if unavailable. Don't fail vault unlock.

---

## Revised Architecture Summary (Post-Round-2 Fixes)

```
Vault File (0600, ~/.local/share/multitop/vault.bin):
┌────────────────────────────────────────────────────────────────────┐
│ Magic | Version: 2 | KeyVer | Created | Counter | Salt            │
├────────────────────────────────────────────────────────────────────┤
│ Argon2id Params: t=10, m=256MiB, p=4 (configurable)               │
├────────────────────────────────────────────────────────────────────┤
│ Wrappers: [                                                         │
│   { type: 0x01, data: SE_wrapped_key },  // macOS only            │
│   { type: 0x03, data: Argon2id_wrapped } // ALWAYS present        │
│ ]                                                                   │
├────────────────────────────────────────────────────────────────────┤
│ Nonce | Ciphertext (AES-256-GCM)                                   │
├────────────────────────────────────────────────────────────────────┤
│ Ed25519 sig (derived from vault_key via HKDF)                     │
└────────────────────────────────────────────────────────────────────┘

Unlock:
1. Verify signature (derived key) → FAIL FAST
2. Try wrappers in priority order (SE → TPM2 → Argon2id)
3. On Argon2id success: rate limit check → re-bind if biometric available
4. Decrypt with vault_key
5. Validate JSON schema
6. Zeroize on drop
```

---

## Updated Effort Estimate

| Component | Round 1 | Round 2 Additions | Total |
|-----------|---------|-------------------|-------|
| Vault format + crypto | 3d | +1d (migration, params in header) | 4d |
| macOS Secure Enclave | 4d | +1d (key invalidation handling) | 5d |
| Linux TPM2/fprintd | 5d | -2d (TPM2 optional, fprintd primary) | 3d |
| Key rotation + rollback | 2d | +1d (migration logic) | 3d |
| File perms + atomic + mlock | 1d | - | 1d |
| Vault API + secrecy | 2d | +1d (rate limiting, schema validation) | 3d |
| Rate limiting + lockout | - | +2d (new) | 2d |
| Settings UI | 2d | +1d (re-bind UI, migration notice) | 3d |
| Upgrade integration | 1d | - | 1d |
| Testing (cross-platform, migration) | 10d | +5d (race conditions, lockout, migration) | 15d |
| **Total** | **~30d** | **+10d** | **~40d (6-8 weeks)** |

---

## Final Recommendation

| Priority | Action |
|----------|--------|
| **P1** | Fix re-bind race condition (file lock or defer) |
| **P1** | Derive Ed25519 signing key from vault_key (HKDF) |
| **P1** | Make Argon2id params configurable + stored in header |
| **P1** | Implement vault migration framework (v1→v2 test in CI) |
| **P2** | Add rate limiting + lockout on password attempts |
| **P2** | Handle Secure Enclave key invalidation gracefully |
| **P2** | Make TPM2 truly optional; fprintd+password primary Linux path |
| **P3** | Verify signature before decrypt; JSON schema validation |

**Bottom line**: The fixed plan is **architecturally sound**. These remaining issues are implementation details that affect robustness and UX, not fundamental cryptographic flaws. Address P1 items before coding; P2 items during implementation.