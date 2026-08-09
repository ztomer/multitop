//! The vault file format's limits, and what happens when they are exceeded.
//!
//! Every bound here is read back off a file that another process, an older
//! build, or a corrupted disk may have written, so each one has to fail as an
//! error rather than as a panic or an over-large allocation.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use multitop_vault::crypto::{Argon2Params, Ed25519PublicKey, Wrapper, WrapperType};
use multitop_vault::format::VaultHeader;
use multitop_vault::VaultError;

const MAX_WRAPPERS: usize = 8;
const MAX_WRAPPER_BYTES: usize = 65535;

const fn key() -> Ed25519PublicKey {
    Ed25519PublicKey([7u8; 32])
}

const fn params() -> Argon2Params {
    Argon2Params {
        t: 1,
        m_kib: 32768,
        p: 1,
    }
}

fn wrapper(kind: WrapperType, len: usize) -> Wrapper {
    Wrapper::new(kind, vec![0u8; len]).expect("a wrapper of this size must be constructible")
}

/// A wrapper past the on-disk limit, built by hand.
///
/// `Wrapper::new` guards the same bound, so this is the only way to reach the
/// checks behind it — and those checks are what stop a wrapper that arrived by
/// another route (deserialisation, a future constructor) from being written
/// with a length field that cannot describe it.
fn oversized(kind: WrapperType) -> Wrapper {
    Wrapper {
        wrapper_type: kind,
        data: vec![0u8; MAX_WRAPPER_BYTES + 1],
    }
}

/// A full header. Only three wrapper types exist, so filling the eight slots
/// means repeating one — which is what the count check counts.
fn eight_wrappers() -> Vec<Wrapper> {
    (0..MAX_WRAPPERS)
        .map(|i| wrapper(WrapperType::Argon2id, 32 + i))
        .collect()
}

// ------------------------------------------------------------ header limits

#[test]
fn a_header_takes_up_to_eight_wrappers_and_no_more() {
    let ok = VaultHeader::new(key(), [1u8; 32], params(), eight_wrappers());
    assert!(ok.is_ok(), "eight wrappers is the documented cap");

    let mut too_many = eight_wrappers();
    too_many.push(wrapper(WrapperType::Argon2id, 32));
    assert!(matches!(
        VaultHeader::new(key(), [1u8; 32], params(), too_many),
        Err(VaultError::TooManyWrappers)
    ));
}

#[test]
fn a_wrapper_larger_than_its_length_field_is_refused() {
    // The on-disk length is a u16; a longer wrapper could not be read back.
    assert!(matches!(
        VaultHeader::new(
            key(),
            [1u8; 32],
            params(),
            vec![oversized(WrapperType::Argon2id)]
        ),
        Err(VaultError::WrapperTooLarge(_))
    ));
    // The constructor guards it too, so the bound holds at both doors.
    assert!(matches!(
        Wrapper::new(WrapperType::Argon2id, vec![0u8; MAX_WRAPPER_BYTES + 1]),
        Err(VaultError::WrapperTooLarge(_))
    ));
}

#[test]
fn the_same_limits_apply_when_the_canary_is_supplied() {
    // Two constructors, one set of rules — the second used to be the one a
    // fresh vault goes through.
    let canary = VaultHeader::generate_canary();

    let mut too_many = eight_wrappers();
    too_many.push(wrapper(WrapperType::Argon2id, 32));
    assert!(matches!(
        VaultHeader::new_with_canary(key(), [1u8; 32], params(), too_many, canary.clone()),
        Err(VaultError::TooManyWrappers)
    ));

    assert!(matches!(
        VaultHeader::new_with_canary(
            key(),
            [1u8; 32],
            params(),
            vec![oversized(WrapperType::Argon2id)],
            canary
        ),
        Err(VaultError::WrapperTooLarge(_))
    ));
}

#[test]
fn adding_a_wrapper_respects_the_cap_unless_it_replaces_one() {
    let mut header = VaultHeader::new(key(), [1u8; 32], params(), eight_wrappers()).unwrap();

    // Full, and this type is not among them: there is nowhere to put it.
    assert!(matches!(
        header.add_wrapper(wrapper(WrapperType::SecureEnclave, 32)),
        Err(VaultError::TooManyWrappers)
    ));

    // Full, but this type is already present: it replaces rather than adds, so
    // the cap is not reached.
    assert!(header
        .add_wrapper(wrapper(WrapperType::Argon2id, 48))
        .is_ok());
    assert_eq!(
        header
            .get_wrapper(WrapperType::Argon2id)
            .map(|w| w.data.len()),
        Some(48),
        "the replacement did not take"
    );

    assert!(matches!(
        header.add_wrapper(oversized(WrapperType::Argon2id)),
        Err(VaultError::WrapperTooLarge(_))
    ));
}

#[test]
fn replacing_a_wrapper_still_refuses_one_that_is_too_large() {
    let mut header = VaultHeader::new(
        key(),
        [1u8; 32],
        params(),
        vec![wrapper(WrapperType::Argon2id, 32)],
    )
    .unwrap();

    assert!(matches!(
        header.replace_wrapper(oversized(WrapperType::Argon2id)),
        Err(VaultError::WrapperTooLarge(_))
    ));
    // The header is left as it was rather than half-updated.
    assert_eq!(
        header
            .get_wrapper(WrapperType::Argon2id)
            .map(|w| w.data.len()),
        Some(32)
    );
}

// ---------------------------------------------------------------- parsing

#[test]
fn a_header_round_trips_through_its_own_bytes() {
    let header = VaultHeader::new(
        key(),
        [9u8; 32],
        params(),
        vec![wrapper(WrapperType::Argon2id, 64)],
    )
    .unwrap();

    let parsed = VaultHeader::from_bytes(&header.to_bytes()).expect("its own bytes must parse");
    assert_eq!(parsed.canary, header.canary);
    assert_eq!(parsed.counter, header.counter);
    assert_eq!(parsed.wrappers.len(), 1);
    assert!(parsed.has_wrapper(WrapperType::Argon2id));
}

#[test]
fn a_file_claiming_more_wrappers_than_the_cap_is_refused_not_allocated_for() {
    let header = VaultHeader::new(
        key(),
        [9u8; 32],
        params(),
        vec![wrapper(WrapperType::Argon2id, 32)],
    )
    .unwrap();
    let mut bytes = header.to_bytes();

    // Find the wrapper-count byte by rewriting it and checking the parse: it
    // is the one whose value is 1 in a header with one wrapper, immediately
    // before the wrapper records. Rather than hardcode the offset, scan for a
    // count that parses today and would over-allocate if trusted.
    let ok = VaultHeader::from_bytes(&bytes).is_ok();
    assert!(ok, "the fixture must parse before it is damaged");

    for i in 0..bytes.len() {
        if bytes[i] != 1 {
            continue;
        }
        let saved = bytes[i];
        bytes[i] = 255;
        let refused = VaultHeader::from_bytes(&bytes);
        bytes[i] = saved;
        if let Err(VaultError::ParseError(msg)) = refused {
            if msg.contains("too many wrappers") {
                return;
            }
        }
    }
    panic!("no wrapper-count byte was validated against the cap");
}

#[test]
fn a_truncated_file_is_refused_rather_than_read_past_its_end() {
    let header = VaultHeader::new(
        key(),
        [9u8; 32],
        params(),
        vec![wrapper(WrapperType::Argon2id, 32)],
    )
    .unwrap();
    let bytes = header.to_bytes();

    for cut in [0usize, 1, 4, 16, bytes.len() / 2, bytes.len() - 1] {
        assert!(
            VaultHeader::from_bytes(&bytes[..cut]).is_err(),
            "a {cut}-byte prefix parsed as a whole header"
        );
    }
}

#[test]
fn a_file_that_is_not_a_vault_is_refused_on_its_magic() {
    let header = VaultHeader::new(
        key(),
        [9u8; 32],
        params(),
        vec![wrapper(WrapperType::Argon2id, 32)],
    )
    .unwrap();
    let mut bytes = header.to_bytes();
    bytes[0] = b'X';
    assert!(VaultHeader::from_bytes(&bytes).is_err());
}

// ------------------------------------------------------------ wrapper types

#[test]
fn wrapper_type_bytes_round_trip_and_unknown_ones_are_refused() {
    for kind in [
        WrapperType::Argon2id,
        WrapperType::SecureEnclave,
        WrapperType::Tpm2,
    ] {
        let byte = kind as u8;
        assert_eq!(WrapperType::from_u8(byte), Some(kind), "byte {byte}");
    }
    // A byte no build knows must not be guessed at: a wrapper decrypted with
    // the wrong scheme is worse than one that will not open.
    assert_eq!(WrapperType::from_u8(200), None);
    assert_eq!(WrapperType::from_u8(255), None);
}

// ------------------------------------------------------- argon2 parameters

#[test]
fn parameters_outside_the_accepted_range_are_refused() {
    // Below the floor is a vault that can be brute-forced; above the ceiling
    // is one whose unlock allocates more memory than the machine has.
    let too_few_iterations = Argon2Params {
        t: 0,
        m_kib: 32768,
        p: 1,
    };
    let err = too_few_iterations.validate().unwrap_err();
    assert!(err.to_string().contains("iterations"), "{err}");

    let no_parallelism = Argon2Params {
        t: 1,
        m_kib: 32768,
        p: 0,
    };
    let err = no_parallelism.validate().unwrap_err();
    assert!(err.to_string().contains("parallelism"), "{err}");

    let too_little_memory = Argon2Params {
        t: 1,
        m_kib: 1,
        p: 1,
    };
    assert!(too_little_memory.validate().is_err());

    assert!(
        params().validate().is_ok(),
        "the documented test params must be legal"
    );
}

#[test]
fn auto_detected_parameters_are_always_legal_on_this_machine() {
    // The detection reads system memory; whatever it finds, the result has to
    // pass the validator that guards every unlock.
    let detected = Argon2Params::auto_detect();
    assert!(
        detected.validate().is_ok(),
        "auto-detection produced parameters the validator rejects: {detected:?}"
    );
    assert!(detected.m_kib >= 32768, "{detected:?}");
}

// ------------------------------------------------------------- the whole file

#[test]
fn a_vault_file_with_a_header_but_no_body_is_refused() {
    // The header parses, and then the file stops. Reading the ciphertext from
    // whatever follows would be reading past the end.
    let header = VaultHeader::new(
        key(),
        [9u8; 32],
        params(),
        vec![wrapper(WrapperType::Argon2id, 32)],
    )
    .unwrap();
    let bytes = header.to_bytes();

    let err = multitop_vault::format::VaultFile::from_bytes(&bytes[..bytes.len() - 1]);
    assert!(err.is_err(), "a file shorter than its own header parsed");
}

#[test]
fn a_wrapper_type_no_build_knows_is_refused_rather_than_guessed_at() {
    // Decrypting a wrapper with the wrong scheme is worse than one that will
    // not open, so an unknown type byte stops the parse.
    let header = VaultHeader::new(
        key(),
        [9u8; 32],
        params(),
        vec![wrapper(WrapperType::Argon2id, 32)],
    )
    .unwrap();
    let mut bytes = header.to_bytes();
    let known = WrapperType::Argon2id as u8;

    for i in 0..bytes.len() {
        if bytes[i] != known {
            continue;
        }
        let saved = bytes[i];
        bytes[i] = 0xEE;
        let refused = VaultHeader::from_bytes(&bytes);
        bytes[i] = saved;
        if matches!(refused, Err(VaultError::InvalidWrapperType(0xEE))) {
            return;
        }
    }
    panic!("no wrapper-type byte was validated");
}
