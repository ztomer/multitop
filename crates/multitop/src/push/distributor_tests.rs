use super::*;

const TOKEN: &str = "tk_SECRETTOKEN000000000000000000";

fn ca_pem() -> String {
    let key = rcgen::KeyPair::generate().expect("key");
    let mut params = rcgen::CertificateParams::new(Vec::<String>::new()).expect("params");
    params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    params.self_signed(&key).expect("ca").pem()
}

fn file(origin: &str, ca: Option<&str>) -> String {
    let base = format!("origin = \"{origin}\"\ntoken = \"{TOKEN}\"\n");
    match ca {
        Some(pem) => format!("{base}ca = \"\"\"{pem}\"\"\"\n"),
        None => base,
    }
}

fn req(url: &str) -> Request {
    Request {
        url: url.into(),
        headers: vec![("ttl".into(), "60".into())],
        body: vec![1],
    }
}

#[test]
fn the_bearer_goes_to_the_origin_and_nowhere_else() {
    let d = Distributor::parse(&file("https://ntfy.example", None)).expect("parses");
    let to = |url: &str| {
        d.authorize(req(url))
            .headers
            .into_iter()
            .find(|(k, _)| k == "authorization")
            .map(|(_, v)| v)
    };
    assert_eq!(
        to("https://ntfy.example/upABC?up=1"),
        Some(format!("Bearer {TOKEN}"))
    );
    assert_eq!(
        to("https://NTFY.example:443/upABC?up=1"),
        Some(format!("Bearer {TOKEN}"))
    );
    for elsewhere in [
        "https://fcm.googleapis.com/v1/projects/x/messages:send",
        "https://ntfy.example.evil/upABC",
        "https://ntfy.example:8443/upABC",
        "http://127.0.0.1/upABC",
    ] {
        assert_eq!(to(elsewhere), None, "{elsewhere}");
    }
}

#[test]
fn authorize_replaces_any_authorization_already_on_the_request() {
    let d = Distributor::parse(&file("https://ntfy.example", None)).expect("parses");
    let mut r = req("https://ntfy.example/upX");
    r.headers
        .push(("Authorization".into(), "Bearer stale".into()));
    let auths: Vec<_> = d
        .authorize(r)
        .headers
        .into_iter()
        .filter(|(k, _)| k.eq_ignore_ascii_case("authorization"))
        .collect();
    assert_eq!(auths.len(), 1);
    assert_eq!(auths[0].1, format!("Bearer {TOKEN}"));
}

#[test]
fn the_ca_is_read_and_debug_never_shows_the_token() {
    let d = Distributor::parse(&file("https://ntfy.example", Some(&ca_pem()))).expect("parses");
    assert_eq!(d.roots().len(), 1);
    let shown = format!("{d:?}");
    assert!(shown.contains("ntfy.example"), "{shown}");
    assert!(!shown.contains("SECRET"), "{shown}");
}

#[test]
fn malformed_files_are_refused_naming_the_field_not_the_value() {
    let cases = [
        ("not toml at all", "TOML"),
        ("origin = \"https://ntfy.example\"\n", "TOML"),
        (
            "origin = \"https://ntfy.example\"\ntoken = \"t\"\nextra = 1\n",
            "TOML",
        ),
        (
            &*file("http://ntfy.example", None),
            "origin: push: plain http",
        ),
        (&*file("https://ntfy.example/up", None), "without a path"),
        (
            "origin = \"https://ntfy.example\"\ntoken = \"\"\n",
            "token: empty",
        ),
        (
            "origin = \"https://ntfy.example\"\ntoken = \"a b\"\n",
            "token: empty or not a printable",
        ),
        (
            &*file("https://ntfy.example", Some("no pem here")),
            "ca: holds no",
        ),
        (
            &*file(
                "https://ntfy.example",
                Some("-----BEGIN CERTIFICATE-----\n!!!\n-----END CERTIFICATE-----"),
            ),
            "ca: is not PEM",
        ),
    ];
    for (text, want) in cases {
        let err = Distributor::parse(text).expect_err(text);
        assert!(err.contains(want), "{text:?} -> {err}");
        assert!(!err.contains("SECRET"), "{err}");
    }
}

#[test]
fn load_is_none_without_a_file_and_refuses_one_open_to_others() {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = tempfile::tempdir().expect("dir");
    let config = dir.path().join("config.toml");
    let path = default_path(&config);
    assert_eq!(path, dir.path().join(DISTRIBUTOR_FILE));
    assert!(load(&path).expect("absent").is_none());

    std::fs::write(&path, file("https://ntfy.example", None)).expect("write");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).expect("chmod");
    assert!(load(&path).is_err(), "readable by others");

    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).expect("chmod");
    let d = load(&path).expect("loads").expect("present");
    assert!(d.serves("https://ntfy.example/upX"));

    std::fs::write(&path, "token = 1").expect("write");
    let err = load(&path).expect_err("malformed");
    assert!(err.contains(DISTRIBUTOR_FILE), "{err}");
}
