use super::*;
use std::os::unix::fs::PermissionsExt as _;

const LAN: &str = "192.168.1.20:8443";

fn token_path(d: &tempfile::TempDir) -> std::path::PathBuf {
    d.path().join("serve-token")
}

fn no_route(_: bool) -> Option<IpAddr> {
    None
}

/// The pairing string in a tty banner, read back by the client's rules.
fn printed_pairing(banner: &str) -> crate::pairing::Fields {
    let line = banner
        .lines()
        .find_map(|l| l.trim().strip_prefix("pair the app: "))
        .unwrap_or_else(|| panic!("no pairing line in:\n{banner}"));
    crate::pairing::check(line).unwrap()
}

#[test]
fn a_loopback_bind_stays_plain_and_makes_no_certificate() {
    let d = tempfile::tempdir().unwrap();
    let ready = prepare(
        "127.0.0.1:8080".parse().unwrap(),
        &token_path(&d),
        true,
        no_route,
    )
    .unwrap();
    assert!(ready.tls.is_none());
    assert!(
        ready.banner.contains("http://127.0.0.1:8080"),
        "{}",
        ready.banner
    );
    assert!(!d.path().join("serve-tls.pem").exists());
}

#[test]
fn a_lan_bind_is_tls_and_prints_a_pairing_the_client_accepts() {
    let d = tempfile::tempdir().unwrap();
    let path = token_path(&d);
    let ready = prepare(LAN.parse().unwrap(), &path, true, no_route).unwrap();
    assert!(ready.tls.is_some(), "a LAN bind without TLS");
    let pem = d.path().join("serve-tls.pem");
    assert_eq!(
        std::fs::metadata(&pem).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert!(
        ready.banner.contains("https://192.168.1.20:8443"),
        "{}",
        ready.banner
    );
    let f = printed_pairing(&ready.banner);
    let id = crate::serve_tls::load_or_create(&pem).unwrap().0;
    assert_eq!((f.host.as_str(), f.port), ("192.168.1.20", 8443));
    assert_eq!(f.token, ready.token);
    assert_eq!(f.sha256, id.fingerprint());
    assert_eq!(f.cert_der, id.cert_der());
    assert!(ready
        .banner
        .contains(&format!("sha256={}", id.fingerprint())));
    assert!(
        ready.banner.contains("--pinnedpubkey sha256//"),
        "{}",
        ready.banner
    );
}

#[test]
fn a_log_gets_the_fingerprint_and_never_the_pairing_string() {
    // The pairing string carries the token: the token's rule applies to it.
    let d = tempfile::tempdir().unwrap();
    let ready = prepare(LAN.parse().unwrap(), &token_path(&d), false, no_route).unwrap();
    assert!(!ready.banner.contains(&ready.token), "{}", ready.banner);
    assert!(
        !ready.banner.contains(crate::pairing::PREFIX),
        "{}",
        ready.banner
    );
    assert!(ready.banner.contains("sha256="), "{}", ready.banner);
    assert!(
        ready.banner.contains("printed only to a terminal"),
        "{}",
        ready.banner
    );
}

#[test]
fn a_restart_keeps_the_certificate_so_the_phone_stays_paired() {
    let d = tempfile::tempdir().unwrap();
    let path = token_path(&d);
    let first = prepare(LAN.parse().unwrap(), &path, true, no_route).unwrap();
    let second = prepare(LAN.parse().unwrap(), &path, true, no_route).unwrap();
    assert_eq!(
        printed_pairing(&first.banner),
        printed_pairing(&second.banner)
    );
    assert!(
        first.banner.contains("pair the phone again"),
        "{}",
        first.banner
    );
    assert!(
        !second.banner.contains("pair the phone again"),
        "{}",
        second.banner
    );
}

#[test]
fn a_wildcard_bind_pairs_at_the_routed_address_or_says_why_not() {
    let d = tempfile::tempdir().unwrap();
    let path = token_path(&d);
    let bind = "0.0.0.0:8443".parse().unwrap();
    let routed = prepare(bind, &path, true, |_| Some(IpAddr::from([10, 0, 0, 7]))).unwrap();
    assert_eq!(printed_pairing(&routed.banner).host, "10.0.0.7");
    let unrouted = prepare(bind, &path, true, no_route).unwrap();
    assert!(unrouted.tls.is_some());
    assert!(
        unrouted.banner.contains("pairing string: unavailable"),
        "{}",
        unrouted.banner
    );
}

#[test]
fn a_hand_written_token_still_serves_tls_and_names_why_it_cannot_pair() {
    let d = tempfile::tempdir().unwrap();
    let path = token_path(&d);
    std::fs::write(&path, "A".repeat(40)).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    let ready = prepare(LAN.parse().unwrap(), &path, true, no_route).unwrap();
    assert!(ready.tls.is_some());
    assert!(
        ready.banner.contains("unavailable -- token"),
        "{}",
        ready.banner
    );
}

#[test]
fn an_identity_others_can_read_refuses_to_start() {
    let d = tempfile::tempdir().unwrap();
    let path = token_path(&d);
    let _ = prepare(LAN.parse().unwrap(), &path, false, no_route).unwrap();
    let pem = d.path().join("serve-tls.pem");
    std::fs::set_permissions(&pem, std::fs::Permissions::from_mode(0o644)).unwrap();
    let Err(e) = prepare(LAN.parse().unwrap(), &path, false, no_route) else {
        panic!("served with a world-readable key");
    };
    assert!(e.contains("chmod 600"), "{e}");
}
