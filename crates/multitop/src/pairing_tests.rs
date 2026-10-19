use super::*;
use std::path::{Path, PathBuf};

const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn addr(s: &str) -> SocketAddr {
    s.parse().unwrap()
}

fn good() -> (String, crate::serve_tls::Identity) {
    let id = crate::serve_tls::generate().unwrap();
    let text = payload(addr("192.168.1.20:8443"), TOKEN, id.cert_der()).unwrap();
    (text, id)
}

#[test]
fn the_server_parses_its_own_payload_by_the_clients_rules() {
    let (text, id) = good();
    let f = check(&text).unwrap();
    assert_eq!(
        f,
        Fields {
            host: "192.168.1.20".into(),
            port: 8443,
            token: TOKEN.into(),
            sha256: id.fingerprint(),
            cert_der: id.cert_der().to_vec(),
        }
    );
    // The field order and spelling the client's tests write by hand.
    let expected_head = format!("multitop://pair?v=1&addr=192.168.1.20:8443&token={TOKEN}&sha256=");
    assert!(text.starts_with(&expected_head), "{text}");
}

#[test]
fn an_ipv6_address_is_bracketed_and_loses_its_scope() {
    let id = crate::serve_tls::generate().unwrap();
    let text = payload(addr("[fd00::20]:8443"), TOKEN, id.cert_der()).unwrap();
    assert_eq!(check(&text).unwrap().host, "fd00::20");
    let scoped = payload(addr("[fe80::1%4]:8443"), TOKEN, id.cert_der()).unwrap();
    assert!(scoped.contains("addr=[fe80::1]:8443"), "{scoped}");
}

#[test]
fn a_token_the_client_would_refuse_yields_no_payload() {
    // A hand-written token file may hold any 32+ visible characters; the pairing
    // string may carry only 64 lowercase hex digits.
    let id = crate::serve_tls::generate().unwrap();
    for token in ["a".repeat(32), TOKEN.to_uppercase(), format!("{TOKEN}00")] {
        let e = payload(addr("192.168.1.20:8443"), &token, id.cert_der()).unwrap_err();
        assert!(e.contains("token"), "{e}");
    }
}

/// Every refusal `PairingPayloadTest.kt` asserts, held against this side's reader:
/// the two must refuse the same strings.
#[test]
fn every_payload_the_client_refuses_is_refused_here() {
    let (text, id) = good();
    let sha = id.fingerprint();
    let other = crate::serve_tls::generate().unwrap().fingerprint();
    let cert = text.rsplit_once("cert=").unwrap().1.to_string();
    let cases = [
        text.replace(&sha, &other),
        text.replace(&cert, &cert[40..]),
        text.replace(&cert, &format!("{cert}==")),
        text.replace(&cert, &format!("{cert}+")),
        text.replace(&format!("&sha256={sha}"), ""),
        format!("{text}&sha256={other}"),
        format!("{text}&insecure=1"),
        text.replace("v=1", "v=2"),
        text.replace("multitop://", "https://"),
        text.replace("//pair?", "//pairing?"),
        text.replace(&sha, &sha[2..]),
        text.replace(&sha, &sha.to_uppercase()),
        text.replace(
            &sha,
            &sha.as_bytes()
                .chunks(2)
                .map(|c| std::str::from_utf8(c).unwrap())
                .collect::<Vec<_>>()
                .join(":"),
        ),
        text.replace(TOKEN, &TOKEN[2..]),
        text.replace(TOKEN, &format!("zz{}", &TOKEN[2..])),
        text.replace("192.168.1.20:8443", "192.168.1.20"),
        text.replace("192.168.1.20:8443", ":8443"),
        text.replace(":8443", ":0"),
        text.replace(":8443", ":65536"),
        text.replace(":8443", ":84x3"),
        text.replace("192.168.1.20", "host name"),
    ];
    for bad in &cases {
        assert!(
            check(bad).is_err(),
            "accepted a payload the client refuses: {bad}"
        );
    }
    for field in FIELDS {
        let without: Vec<&str> = text
            .strip_prefix(PREFIX)
            .unwrap()
            .split('&')
            .filter(|p| !p.starts_with(&format!("{field}=")))
            .collect();
        let e = check(&format!("{PREFIX}{}", without.join("&"))).unwrap_err();
        assert!(e.contains(field), "{e}");
    }
    assert!(
        check(&format!("  {text}\n")).is_ok(),
        "surrounding whitespace from a paste"
    );
}

#[test]
fn an_unspecified_bind_advertises_the_routed_address() {
    let lan: IpAddr = "192.168.1.20".parse().unwrap();
    let bind = addr("0.0.0.0:8443");
    assert_eq!(
        advertised(bind, |_| Some(lan)),
        Some(addr("192.168.1.20:8443"))
    );
    assert_eq!(advertised(bind, |_| None), None);
    assert_eq!(
        advertised(bind, |_| Some(IpAddr::from([127, 0, 0, 1]))),
        None
    );
    let v6 = addr("[::]:8443");
    assert_eq!(
        advertised(v6, |is_v6| is_v6.then_some(lan)),
        Some(addr("192.168.1.20:8443"))
    );
    let named = addr("10.0.0.5:9000");
    assert_eq!(
        advertised(named, |_| panic!("a named bind needs no route")),
        Some(named)
    );
}

#[test]
fn the_route_probe_never_answers_unspecified() {
    // Whatever this machine's network, the probe either names an address or says
    // it has none; it never hands `advertised` the wildcard back.
    for v6 in [false, true] {
        assert!(route_source(v6).is_none_or(|ip| !ip.is_unspecified()));
    }
}

#[test]
fn a_real_payload_fits_one_qr_code_a_terminal_can_show() {
    let (text, _) = good();
    let code = qr(&text).unwrap();
    let widths: Vec<usize> = code.lines().map(|l| l.chars().count()).collect();
    multitop_testassert::assert_not_empty!(widths);
    assert!(widths.iter().all(|w| *w == widths[0]), "ragged QR rows");
    // Measured 2026-10-18: a P-256 payload is a version-19 code, 93 modules plus
    // the quiet zone. Wider than this and an ordinary terminal wraps it.
    assert!(widths[0] <= 120, "{} columns", widths[0]);
    assert!(qr("x".repeat(8000).as_str()).is_err());
}

// ── The cross-language contract ──────────────────────────────────────────────

fn golden(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../android/app/src/test/resources")
        .join(name)
}

const GOLDEN_ADDR: &str = "192.168.1.20:8443";

/// The pairing string the Kotlin tests parse (`ServerContractTest.kt`) is exactly
/// what this module prints for the certificate beside it. Regenerate both with
/// `MULTITOP_REGEN_PAIRING_GOLDEN=1 cargo test -p multitop golden` -- after a change
/// to [`crate::serve_tls::generate`], so the Kotlin side tests the new shape.
#[test]
fn the_golden_pairing_string_is_what_the_server_prints() {
    let pem_path = golden("rust_server.pem");
    let txt_path = golden("rust_server_pairing.txt");
    if std::env::var_os("MULTITOP_REGEN_PAIRING_GOLDEN").is_some() {
        // The secrets gate refuses a private key in the tree unless the line above
        // it says why it may stand; PEM readers skip text outside the blocks.
        let pem = crate::serve_tls::generate().unwrap().to_pem().replace(
            "-----BEGIN PRIVATE KEY",
            "secret-ok: a test fixture's key; nothing outside the tests trusts its certificate\n-----BEGIN PRIVATE KEY",
        );
        std::fs::write(&pem_path, pem).unwrap();
    }
    let id = crate::serve_tls::parse(&std::fs::read(&pem_path).unwrap(), &pem_path).unwrap();
    let printed = payload(addr(GOLDEN_ADDR), TOKEN, id.cert_der()).unwrap();
    if std::env::var_os("MULTITOP_REGEN_PAIRING_GOLDEN").is_some() {
        std::fs::write(&txt_path, format!("{printed}\n")).unwrap();
    }
    let on_disk = std::fs::read_to_string(&txt_path).unwrap();
    assert_eq!(
        on_disk.trim_end(),
        printed,
        "regenerate: see this test's doc"
    );
}

/// The QR code `--serve` draws for the golden pairing string, exactly as the
/// terminal receives it. The Kotlin scanner tests (`QrDecoderTest.kt`) paint
/// these glyphs the way a dark AND a light terminal would and decode both, so
/// the app is held to the code the server really draws, in both polarities.
/// Regenerated with the pairing string (see that test's doc).
#[test]
fn the_golden_pairing_qr_is_what_the_server_draws() {
    let printed = std::fs::read_to_string(golden("rust_server_pairing.txt")).unwrap();
    let drawn = qr(printed.trim_end()).unwrap();
    let qr_path = golden("rust_server_pairing_qr.txt");
    if std::env::var_os("MULTITOP_REGEN_PAIRING_GOLDEN").is_some() {
        std::fs::write(&qr_path, format!("{drawn}\n")).unwrap();
    }
    let on_disk = std::fs::read_to_string(&qr_path).unwrap();
    assert_eq!(
        on_disk.trim_end_matches('\n'),
        drawn,
        "regenerate: see the_golden_pairing_string_is_what_the_server_prints"
    );
}
