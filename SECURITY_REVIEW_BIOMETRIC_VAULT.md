# Security Review: Pragmatic Biometric Vault Plan

**Reviewer**: Security Expert Persona
**Date**: 2026-07-30
**Verdict**: **CONDITIONAL PASS** — Fix critical issues before implementation

---

## Executive Summary

The plan is pragmatic and usable, but has **three critical vulnerabilities** and several **significant weaknesses** that must be addressed. The fallback to system password is a sensible UX decision, but the cryptographic binding between biometric and password paths creates a downgrade attack surface.

---

## CRITICAL Issues (Must Fix)

### 1. Downgrade Attack: Biometric → Password Path

**Vulnerability**: An attacker with local access can force the password path by:
- Blocking biometric hardware (unplugging fingerprint reader, covering Touch ID sensor)
- Triggering biometric failure (3 failed attempts locks Touch ID until password entered)
- Simply selecting "Use Password" in the UI

**Impact**: The vault's security reduces entirely to `Argon2id(password)`. The biometric binding provides **zero additional security** against a determined local attacker — it only adds convenience.

**Fix**: 
- Make biometric binding **cryptographically mandatory** for vault creation on supported hardware
- If biometric hardware exists, vault key = `HKDF(biometric_secret, salt)` ONLY
- Password path derives a **different key** that cannot decrypt biometric-created vaults
- On first unlock with password, **re-encrypt** vault with biometric-bound key (if hardware available)

### 2. Argon2id Parameters Insufficient for "System Password"

**Vulnerability**: `t=3, m=64MiB, p=4` (~1 second) is calibrated for **user-chosen passphrases** with entropy ~40-60 bits. But the "system password" here is the **sudo password** — often:
- Short (8-12 chars)
- Low entropy (dictionary words, patterns)
- Reused across systems
- Known to attacker (shoulder surfed, keylogged, extracted from memory)

**Impact**: Offline attack on vault file:
- Attacker copies `vault.pqenc` → offline dictionary attack
- 64 MiB / 3 iterations = ~200M guesses/second on GPU
- 8-char alphanumeric = 62^8 ≈ 2^47 → **cracked in hours**

**Fix**:
- **Minimum**: `t=10, m=256MiB, p=4` (~3-5 seconds) — still acceptable UX
- **Better**: Use **scrypt** with `N=2^20, r=8, p=1` (~256 MiB, sequential memory-hard)
- **Best**: Require **user-chosen high-entropy passphrase** at vault creation, separate from sudo password
- **Acceptable compromise**: Document that vault security = sudo password entropy; recommend `sudo` password ≥ 16 chars random

### 3. Vault File Permissions & Location

**Vulnerability**: `~/.config/multitop/vault.pqenc` is world-readable by default (config dir typically 755).

**Impact**: Any local user/process can copy the vault file for offline attack.

**Fix**:
- Set file permissions to `0600` on creation (`std::fs::set_permissions`)
- Store in `~/.local/share/multitop/` (XDG_DATA_HOME) with `0700` directory
- Consider `mlock`ing the decrypted passwords in memory (prevent swap)

---

## HIGH Issues

### 4. No Forward Secrecy / Key Rotation

**Issue**: Vault encrypted with single key forever. If key compromised (memory dump, cold boot, future cryptanalysis), **all historical passwords exposed**.

**Fix**:
- Versioned key derivation: `key_v{N} = HKDF(master_secret, "vault-key-v{N}")`
- On password change: re-encrypt vault with new key version
- On biometric re-enrollment: rotate key
- Store key version in vault header

### 5. Ed25519 Signature Provides False Confidence

**Issue**: Ed25519 signature on vault file only detects **tampering after creation**. It does NOT prevent:
- Attacker replacing entire vault file (if they have write access to config dir)
- Attacker rolling back to old vault version (downgrade attack)
- Malicious multitop binary signing attacker's vault

**Fix**:
- Store **creation timestamp** + **monotonic counter** in signed header
- Reject vaults with timestamp older than last known good (stored in keychain/TPM)
- Better: Use **age** or **libsodium secretbox** which includes authentication

### 6. Biometric Secret Handling on macOS

**Issue**: `SecKey.unwrap_symmetric_key()` requires biometric **every time**. But the plan says "derive vault key from biometric secret" — this implies the biometric secret is a stable key in Secure Enclave.

**Reality**: Secure Enclave keys are **not extractable**. You can't get a "biometric secret" bytes to feed to HKDF. You can only:
- **Sign** data with the key (requires biometric each time)
- **Unwrap** a previously wrapped key (requires biometric each time)

**Correct approach**:
1. Vault creation: Generate random `vault_key`. Wrap it with Secure Enclave key → `wrapped_key`. Store `wrapped_key` in vault header.
2. Unlock: Biometric → Secure Enclave unwraps `wrapped_key` → `vault_key` → decrypt vault.
3. Password fallback: `vault_key` encrypted with `Argon2id(password)` → `wrapped_key_password`. Store both wrapped forms.

### 7. No Protection Against Malicious Multitop Binary

**Issue**: If attacker replaces `multitop` binary (supply chain, local privilege escalation), they can:
- Log vault password
- Exfiltrate decrypted passwords
- Skip biometric check entirely

**Fix**: Not solvable at application level. Document: "Vault security assumes untampered binary. Verify binary signature (`codesign -vv` / `sha256sum`) before running."

---

## MEDIUM Issues

### 8. JSON Parsing of Decrypted Vault

**Risk**: Decrypt → `serde_json::from_str` → potential DoS via malicious ciphertext (if auth tag bypassed somehow).

**Fix**: Use `serde_json::from_slice` with size limits. Validate schema strictly.

### 9. Password in Memory During Upgrade

**Risk**: Sudo passwords held in `HashMap<String, String>` during upgrade session. If process memory dumped (coredump, `/proc/pid/mem`, debugger), all passwords exposed.

**Fix**:
- Use `zeroize::Zeroize` on drop
- Clear passwords from map after `spawn_upgrade` reads them
- Consider `secrecy::SecretString` wrapper

### 10. Argon2id Salt Reuse

**Issue**: Single salt per vault. If user creates multiple vaults (different machines), same password → same key if salt reused.

**Fix**: Salt = random 32 bytes per vault (already in plan — verify implementation).

### 11. No Integrity Check on Password Entries

**Issue**: Vault stores `{"host": "password"}`. If attacker can flip bits in ciphertext (AES-GCM prevents this, but if implementation error...), password corruption could cause silent failures or injection.

**Fix**: AES-GCM provides integrity. Ensure nonce never reused (random 12 bytes per encryption).

---

## LOW / INFORMATIONAL

### 12. Biometric Availability Detection Race

**Issue**: `try_biometric_unlock()` may succeed on first call but fail on subsequent (sensor busy, timeout).

**Fix**: Cache biometric availability at startup; retry with exponential backoff.

### 13. Linux fprintd Not Universal

**Issue**: Many distros lack fprintd, or have broken drivers. Plan says "skip biometric on Linux" — acceptable but document clearly.

### 14. No Vault Backup/Recovery

**Issue**: If vault file corrupted/deleted, all passwords lost. No recovery mechanism.

**Fix**: 
- Export encrypted backup (password-protected) to user-chosen location
- Document: "Vault is device-local. Re-enter passwords on new machine."

---

## REVISED ARCHITECTURE (Post-Fix)

```
Vault File (0600, ~/.local/share/multitop/vault.bin):
┌────────────────────────────────────────────────────────────┐
│ Magic: "MQV1" | Ver: 1 | KeyVer: 3                         │
├────────────────────────────────────────────────────────────┤
│ Salt: 32 bytes                                             │
├────────────────────────────────────────────────────────────┤
│ Wrapped Keys (choose one at unlock):                       │
│   0x01: SE_wrapped_key (32 bytes + tag)  ← macOS biometric │
│   0x02: TPM2_wrapped_key (variable)       ← Linux TPM2     │
│   0x03: Argon2id_wrapped_key (32 bytes)   ← password       │
├────────────────────────────────────────────────────────────┤
│ Created: u64 timestamp | Counter: u32                      │
├────────────────────────────────────────────────────────────┤
│ Nonce: 12 bytes                                            │
├────────────────────────────────────────────────────────────┤
│ Ciphertext: AES-256-GCM(vault_key, JSON)                   │
├────────────────────────────────────────────────────────────┤
│ Ed25519 Signature (over header || ciphertext)              │
└────────────────────────────────────────────────────────────┘

Unlock Flow:
1. Detect available wrapped key types (prefer biometric/TPM)
2. Try biometric/TPM unwrap → vault_key
3. If unavailable/failed → prompt password → Argon2id unwrap → vault_key
4. Decrypt + verify signature + verify counter > last_known
5. On success: if used password path AND biometric available → 
   re-wrap vault_key with biometric → update vault file
```

---

## Final Recommendation

| Priority | Action |
|----------|--------|
| **P0** | Fix downgrade attack: biometric and password paths must use **independent key wrapping**, not shared derivation |
| **P0** | Increase Argon2id to `t=10, m=256MiB` minimum; document password entropy requirements |
| **P0** | File permissions `0600` + `0700` directory; `mlock` decrypted passwords |
| **P1** | Implement key versioning + counter for rollback protection |
| **P1** | Correct macOS Secure Enclave usage (wrap/unwrap, not raw secret extraction) |
| **P2** | Add vault export/backup feature |
| **P2** | Use `secrecy::SecretString` + `zeroize` for all password handling |
| **P3** | Binary integrity verification documentation |

**Bottom line**: The plan is **salvageable** but the current crypto design has a **fundamental flaw** (downgrade attack) that makes biometric security theater. Fix the key wrapping architecture first, then implement.