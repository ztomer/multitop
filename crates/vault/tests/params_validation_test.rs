//! Argon2 parameters are untrusted input and must be bounded.
//!
//! The vault header is parsed *before* the KDF runs, which is before anything
//! has been authenticated -- the signature over the header can only be checked
//! with a key that the KDF has not derived yet. So `m_kib` out of the file
//! directly sizes a multi-gigabyte allocation with nothing vouching for it.
//! `wrapper_count` in the same header was bounds-checked; this field, the one
//! that actually sizes an allocation, was not.
//!
//! Separately, `from_config` multiplied MiB by 1024 in `u32` *before* clamping,
//! so asking for the documented maximum wrapped to zero and clamped up to the
//! documented minimum: the strongest setting produced the weakest KDF.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use multitop_vault::crypto::Argon2Params;

#[test]
fn hostile_memory_cost_is_rejected_not_allocated() {
    // A corrupt or hostile vault.bin claiming 4 TiB of Argon2 memory.
    let hostile = Argon2Params {
        t: 1,
        m_kib: u32::MAX,
        p: 1,
    };
    assert!(
        hostile.to_argon2().is_err(),
        "m_kib=u32::MAX must be refused before it reaches the allocator"
    );
}

#[test]
fn trivially_weak_parameters_are_rejected() {
    // Below the documented 32 MiB floor: not a real Argon2 cost.
    let weak = Argon2Params {
        t: 1,
        m_kib: 8,
        p: 1,
    };
    assert!(weak.to_argon2().is_err(), "m_kib=8 is below the floor");

    let no_iterations = Argon2Params {
        t: 0,
        m_kib: 32_768,
        p: 1,
    };
    assert!(no_iterations.to_argon2().is_err(), "t=0 is not a cost");
}

#[test]
fn documented_bounds_are_accepted() {
    for (t, m_kib, p) in [(1u8, 32_768u32, 1u8), (20, 4_194_304, 8), (3, 262_144, 4)] {
        let params = Argon2Params { t, m_kib, p };
        assert!(
            params.to_argon2().is_ok(),
            "documented-range params must still work: t={t} m_kib={m_kib} p={p}"
        );
    }
}

#[test]
fn asking_for_maximum_memory_does_not_yield_minimum() {
    // The documented unit is MiB. These are absurd values in that unit, but a
    // fat-finger or a MiB/KiB mix-up produces them, and the clamp exists
    // precisely so absurd input lands on a sane value.
    for m_mib in [4_194_304u32, 8_388_608, u32::MAX] {
        let params = Argon2Params::from_config(3, m_mib, 4);
        assert_eq!(
            params.m_kib,
            4_194_304,
            "over-large request must clamp UP to the 4 GiB ceiling, not wrap to the 32 MiB floor \
             (m_mib={m_mib} gave {} MiB)",
            params.m_kib / 1024
        );
    }
}

#[test]
fn ordinary_config_values_are_unchanged() {
    assert_eq!(Argon2Params::from_config(3, 128, 4).m_kib, 131_072);
    assert_eq!(Argon2Params::from_config(3, 4096, 4).m_kib, 4_194_304);
    // Under the floor still clamps up.
    assert_eq!(Argon2Params::from_config(3, 1, 4).m_kib, 32_768);
}

// ---------------------------------------------------------------------------
// Nonce uniqueness
// ---------------------------------------------------------------------------

/// The vault key is stable across saves, so a repeated AES-GCM nonce under that
/// key is the catastrophic case: it leaks the XOR of two plaintexts and breaks
/// the authentication guarantee outright. Every save must draw a fresh nonce.
#[test]
fn every_save_draws_a_fresh_nonce() {
    let dir = std::env::temp_dir().join(format!("multitop_nonce_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let vault_path = dir.join("vault.bin");

    let vault = multitop_vault::Vault::new(multitop_vault::VaultConfig {
        vault_path: vault_path.clone(),
        argon2_params: Some(Argon2Params {
            t: 1,
            m_kib: 32_768,
            p: 1,
        }),
        use_os_keychain: false,
    });
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(vault.initialize("master-pw"))
        .unwrap();

    let mut seen = std::collections::HashSet::new();
    let mut unlocked = vault.unlock_with_password("master-pw").unwrap();
    for i in 0..64 {
        unlocked
            .set_password(
                format!("user@host-{i}:22"),
                &secrecy::SecretString::from(format!("pw-{i}")),
            )
            .unwrap();
        let nonce = read_header_nonce(&vault_path);
        assert!(
            seen.insert(nonce),
            "save {i} reused an AES-GCM nonce under the same vault key"
        );
    }
    assert_eq!(
        seen.len(),
        64,
        "every save must contribute a distinct nonce"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// Pull the 12-byte nonce out of the on-disk header by re-parsing it, rather
/// than by hardcoding a byte offset that would silently drift if the header
/// layout changed.
fn read_header_nonce(path: &std::path::Path) -> [u8; 12] {
    let bytes = std::fs::read(path).unwrap();
    let header = multitop_vault::format::VaultHeader::from_bytes(&bytes).unwrap();
    header.nonce
}
