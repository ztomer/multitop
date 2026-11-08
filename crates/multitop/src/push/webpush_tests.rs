use super::*;
use base64::Engine as _;

fn b64(s: &str) -> Vec<u8> {
    let clean: String = s.split_whitespace().collect();
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(clean)
        .unwrap_or_default()
}

/// RFC 8291 Appendix A, every input and the published result.
struct Vector {
    plaintext: Vec<u8>,
    as_public: Vec<u8>,
    ua_public: Vec<u8>,
    salt: [u8; SALT_LEN],
    auth: Vec<u8>,
    ecdh: Vec<u8>,
    header: Vec<u8>,
    ciphertext: Vec<u8>,
}

fn rfc8291() -> Vector {
    let mut salt = [0u8; SALT_LEN];
    salt.copy_from_slice(&b64("DGv6ra1nlYgDCS1FRnbzlw"));
    Vector {
        plaintext: b64("V2hlbiBJIGdyb3cgdXAsIEkgd2FudCB0byBiZSBhIHdhdGVybWVsb24"),
        as_public: b64("BP4z9KsN6nGRTbVYI_c7VJSPQTBtkgcy27mlmlMoZIIg
            Dll6e3vCYLocInmYWAmS6TlzAC8wEqKK6PBru3jl7A8"),
        ua_public: b64("BCVxsr7N_eNgVRqvHtD0zTZsEc6-VV-
            JvLexhqUzORcx aOzi6-AYWXvTBHm4bjyPjs7Vd8pZGH6SRpkNtoIAiw4"),
        salt,
        auth: b64("BTBZMqHH6r4Tts7J_aSIgg"),
        ecdh: b64("kyrL1jIIOHEzg3sM2ZWRHDRB62YACZhhSlknJ672kSs"),
        header: b64(
            "DGv6ra1nlYgDCS1FRnbzlwAAEABBBP4z 9KsN6nGRTbVYI_c7VJSPQTBtkgcy27ml
            mlMoZIIgDll6e3vCYLocInmYWAmS6Tlz AC8wEqKK6PBru3jl7A8",
        ),
        ciphertext: b64(
            "8pfeW0KbunFT06SuDKoJH9Ql87S1QUrd irN6GcG7sFz1y1sqLgVi1VhjVkHsUoEs
            bI_0LpXMuGvnzQ",
        ),
    }
}

#[test]
fn seal_reproduces_the_rfc_8291_appendix_a_message_byte_for_byte() {
    let v = rfc8291();
    assert_eq!(v.header.len(), 86, "the vector decoded");
    let body = seal(
        &v.ecdh,
        &v.ua_public,
        &v.as_public,
        &v.auth,
        &v.salt,
        &v.plaintext,
    )
    .expect("the RFC's inputs seal");
    let mut want = v.header.clone();
    want.extend_from_slice(&v.ciphertext);
    assert_eq!(body, want);
}

#[test]
fn seal_refuses_keys_of_the_wrong_shape_and_an_oversized_payload() {
    let v = rfc8291();
    assert_eq!(
        seal(
            &v.ecdh,
            &v.ua_public[1..],
            &v.as_public,
            &v.auth,
            &v.salt,
            b"x"
        ),
        Err(SealError::BadPublicKey)
    );
    assert_eq!(
        seal(
            &v.ecdh,
            &v.ua_public,
            &v.as_public,
            &v.auth[1..],
            &v.salt,
            b"x"
        ),
        Err(SealError::BadAuth)
    );
    let big = vec![b'a'; MAX_PLAINTEXT + 1];
    assert_eq!(
        seal(&v.ecdh, &v.ua_public, &v.as_public, &v.auth, &v.salt, &big),
        Err(SealError::TooLarge)
    );
    let fits = vec![b'a'; MAX_PLAINTEXT];
    assert!(seal(&v.ecdh, &v.ua_public, &v.as_public, &v.auth, &v.salt, &fits).is_ok());
    assert_eq!(
        encrypt(&[4u8; P256_POINT_LEN], &v.auth, b"x"),
        Err(SealError::BadPublicKey),
        "a point not on the curve is refused by the agreement"
    );
    for e in [
        SealError::BadPublicKey,
        SealError::BadAuth,
        SealError::TooLarge,
        SealError::Crypto,
    ] {
        multitop_testassert::assert_not_empty!(e.to_string());
    }
}

#[test]
fn encrypt_agrees_with_the_receivers_key_and_names_its_own_in_the_header() {
    // The receiver's side, independently: its own key pair, and the ECDH it
    // computes from the as_public the header carries.
    let rng = ring::rand::SystemRandom::new();
    let ua = agreement::EphemeralPrivateKey::generate(&agreement::ECDH_P256, &rng).expect("key");
    let ua_public = ua.compute_public_key().expect("public").as_ref().to_vec();
    let auth = [7u8; AUTH_LEN];
    let body = encrypt(&ua_public, &auth, b"host down").expect("encrypts");

    let mut salt = [0u8; SALT_LEN];
    salt.copy_from_slice(&body[..SALT_LEN]);
    let rs = u32::from_be_bytes(body[SALT_LEN..SALT_LEN + 4].try_into().expect("4 bytes"));
    assert_eq!(rs, RECORD_SIZE);
    assert_eq!(usize::from(body[SALT_LEN + 4]), P256_POINT_LEN);
    let as_public = &body[SALT_LEN + 5..SALT_LEN + 5 + P256_POINT_LEN];
    let peer = agreement::UnparsedPublicKey::new(&agreement::ECDH_P256, as_public);
    let ecdh = agreement::agree_ephemeral(ua, &peer, <[u8]>::to_vec).expect("agrees");
    // Same secret on both sides means the same body for the same salt.
    let again = seal(&ecdh, &ua_public, as_public, &auth, &salt, b"host down").expect("seals");
    assert_eq!(body, again);
    // And two deliveries never share a salt or a key.
    let other = encrypt(&ua_public, &auth, b"host down").expect("encrypts");
    assert_ne!(
        body[..SALT_LEN + 5 + P256_POINT_LEN],
        other[..SALT_LEN + 5 + P256_POINT_LEN]
    );
}
