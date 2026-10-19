use super::*;
use std::os::unix::fs::PermissionsExt as _;

fn dir() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

fn mode(path: &Path) -> u32 {
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

/// The DER of a critical `basicConstraints` (2.5.29.19) whose value is an empty
/// SEQUENCE: `CA:FALSE` written out, as `openssl x509 -text` reads it.
const CA_FALSE: [u8; 12] = [
    0x06, 0x03, 0x55, 0x1d, 0x13, 0x01, 0x01, 0xff, 0x04, 0x02, 0x30, 0x00,
];

#[test]
fn a_generated_identity_is_a_leaf_with_a_hex_fingerprint() {
    let id = generate().unwrap();
    let fp = id.fingerprint();
    assert_eq!(fp.len(), 64);
    assert!(
        fp.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')),
        "{fp}"
    );
    let der = id.cert_der();
    assert!(
        der.windows(CA_FALSE.len()).any(|w| w == CA_FALSE),
        "no explicit CA:FALSE in the certificate"
    );
    id.server_config().unwrap();
}

#[test]
fn the_pem_round_trips_to_the_same_certificate() {
    let id = generate().unwrap();
    let back = parse(id.to_pem().as_bytes(), Path::new("x.pem")).unwrap();
    assert_eq!(back.cert_der(), id.cert_der());
    assert_eq!(back.curl_pin(), id.curl_pin());
}

#[test]
fn the_identity_file_is_created_owner_only_and_reused() {
    // A new certificate is a new pairing, so a restart must serve the same one --
    // and the file holds the private key, so its mode is the whole point.
    let d = dir();
    let path = d.path().join("nested/serve-tls.pem");
    let (first, created) = load_or_create(&path).unwrap();
    assert!(created);
    assert_eq!(mode(&path), 0o600);
    assert_eq!(mode(&d.path().join("nested")), 0o700);
    let (second, created) = load_or_create(&path).unwrap();
    assert!(!created, "the second start claims it created the file");
    assert_eq!(first.fingerprint(), second.fingerprint());
}

#[test]
fn an_identity_file_others_can_open_is_refused() {
    let d = dir();
    let path = d.path().join("serve-tls.pem");
    let _ = load_or_create(&path).unwrap();
    for open in [0o644, 0o640, 0o604, 0o620] {
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(open)).unwrap();
        let Err(e) = load_or_create(&path) else {
            panic!("mode {open:o} was accepted");
        };
        assert!(e.contains("chmod 600"), "{e}");
    }
}

#[test]
fn a_damaged_identity_is_refused_never_replaced() {
    // Silently making a new one would unpair the phone without a word.
    let d = dir();
    let path = d.path().join("serve-tls.pem");
    let a = generate().unwrap().to_pem();
    let b = generate().unwrap().to_pem();
    let cert_of_a = &a[..a.find("-----BEGIN PRIVATE KEY").unwrap()];
    let key_of_b = &b[b.find("-----BEGIN PRIVATE KEY").unwrap()..];
    for body in [
        String::from("not pem"),
        cert_of_a.to_string(),
        format!("{cert_of_a}{key_of_b}"),
        format!("{cert_of_a}{a}"),
    ] {
        let _ = std::fs::remove_file(&path);
        std::fs::write(&path, &body).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        let Err(e) = load_or_create(&path) else {
            panic!("accepted {body:?}");
        };
        assert!(e.contains("delete it"), "{e}");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            body,
            "the file was rewritten"
        );
    }
}

#[test]
fn only_loopback_binds_stay_plain() {
    for plain in ["127.0.0.1:8080", "[::1]:8080"] {
        assert!(!needs_tls(plain.parse().unwrap()), "{plain}");
    }
    for tls in [
        "0.0.0.0:8080",
        "[::]:8080",
        "192.168.1.20:8443",
        "[fd00::20]:8443",
    ] {
        assert!(needs_tls(tls.parse().unwrap()), "{tls}");
    }
}

#[test]
fn the_identity_lives_beside_the_token() {
    let token = Path::new("/home/u/.config/multitop/serve-token");
    assert_eq!(
        default_path(token),
        Path::new("/home/u/.config/multitop/serve-tls.pem")
    );
}
