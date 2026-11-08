use super::http::{Pending, Request, Response, Sender};
use super::*;
use std::sync::Mutex;

/// Answers each request with the next scripted status (or an error), and
/// records the request.
struct Scripted {
    answers: Mutex<Vec<Result<u16, String>>>,
    seen: Mutex<Vec<Request>>,
}

impl Scripted {
    fn new(answers: Vec<Result<u16, String>>) -> Arc<Self> {
        Arc::new(Self {
            answers: Mutex::new(answers.into_iter().rev().collect()),
            seen: Mutex::new(vec![]),
        })
    }
    fn hosts(&self) -> Vec<String> {
        self.seen
            .lock()
            .expect("lock")
            .iter()
            .map(|r| {
                r.url
                    .parse::<hyper::Uri>()
                    .ok()
                    .and_then(|u| u.host().map(str::to_string))
                    .unwrap_or_default()
            })
            .collect()
    }
}

impl Sender for Scripted {
    fn post(&self, req: Request) -> Pending<'_> {
        let is_token = req.url.contains("oauth2");
        self.seen.lock().expect("lock").push(req);
        let next = if is_token {
            Ok(200)
        } else {
            self.answers.lock().expect("lock").pop().unwrap_or(Ok(200))
        };
        Box::pin(async move {
            next.map(|status| Response {
                status,
                body: if is_token {
                    br#"{"access_token":"synthetic-access","expires_in":3599}"#.to_vec()
                } else {
                    vec![]
                },
            })
        })
    }
}

fn b64(x: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(x)
}

fn up() -> Registration {
    let rng = ring::rand::SystemRandom::new();
    let key = ring::agreement::EphemeralPrivateKey::generate(&ring::agreement::ECDH_P256, &rng)
        .expect("key");
    Registration::UnifiedPush {
        endpoint: "https://ntfy.example/upSYNTH?up=1".into(),
        p256dh: b64(key.compute_public_key().expect("pub").as_ref()),
        auth: b64(&[1u8; 16]),
    }
}

fn fcm_reg() -> Registration {
    Registration::Fcm {
        token: "cSyntheticFid_1-abcdef".into(),
    }
}

fn fcm_client() -> fcm::Fcm {
    fcm::Fcm::new(fcm::tests::account()).expect("test account")
}

fn alert() -> Alert {
    Alert::new(
        Kind::Down,
        "web",
        "web is unreachable".into(),
        "lost".into(),
    )
}

#[test]
fn plan_puts_unifiedpush_first_and_fcm_only_when_both_sides_have_it() {
    let both = Registrations {
        unifiedpush: Some(up()),
        fcm: Some(fcm_reg()),
    };
    let t = |p: Vec<Registration>| p.iter().map(Registration::transport).collect::<Vec<_>>();
    assert_eq!(
        t(plan(&both, true)),
        vec![Transport::UnifiedPush, Transport::Fcm]
    );
    assert_eq!(
        t(plan(&both, false)),
        vec![Transport::UnifiedPush],
        "no service account, no FCM"
    );
    let only_fcm = Registrations {
        unifiedpush: None,
        fcm: Some(fcm_reg()),
    };
    assert_eq!(t(plan(&only_fcm, true)), vec![Transport::Fcm]);
    multitop_testassert::assert_empty!(plan(&only_fcm, false));
    multitop_testassert::assert_empty!(plan(&Registrations::default(), true));
}

#[test]
fn registrations_are_validated_before_they_are_kept() {
    assert_eq!(up().validate(), Ok(()));
    assert_eq!(fcm_reg().validate(), Ok(()));
    let bad = |endpoint: &str, p256dh: String, auth: String| Registration::UnifiedPush {
        endpoint: endpoint.into(),
        p256dh,
        auth,
    };
    let key = b64(&[4u8; 65]);
    let auth = b64(&[0u8; 16]);
    assert!(bad("http://ntfy.example/up", key.clone(), auth.clone())
        .validate()
        .is_err());
    assert!(bad(
        &format!("https://x/{}", "a".repeat(MAX_REGISTRATION_LEN)),
        key.clone(),
        auth.clone()
    )
    .validate()
    .is_err());
    assert!(bad("https://x/up", b64(&[4u8; 64]), auth.clone())
        .validate()
        .is_err());
    assert!(bad("https://x/up", key, b64(&[0u8; 15]))
        .validate()
        .is_err());
    assert!(bad("https://x/up", "%%%".into(), auth).validate().is_err());
    for token in [
        "",
        "has space",
        "quote\"",
        &"a".repeat(MAX_REGISTRATION_LEN + 1),
    ] {
        assert!(
            Registration::Fcm {
                token: token.into()
            }
            .validate()
            .is_err(),
            "{token:?}"
        );
    }
}

#[test]
fn registration_json_is_tagged_by_kind_and_never_debug_prints_a_secret() {
    let reg: Registration =
        serde_json::from_str(r#"{"kind":"fcm","token":"SECRETTOKEN"}"#).expect("parses");
    assert_eq!(reg.transport(), Transport::Fcm);
    assert_eq!(format!("{reg:?}"), "fcm");
    assert_eq!(format!("{:?}", up()), "unifiedpush");
    assert!(serde_json::from_str::<Registration>(r#"{"kind":"apns","token":"x"}"#).is_err());
    assert!(
        serde_json::from_str::<Registration>(r#"{"kind":"fcm","token":"x","extra":1}"#).is_err()
    );
}

#[test]
fn the_unifiedpush_request_is_web_push_ciphertext_to_the_endpoint() {
    let req = unifiedpush_request(&up(), &alert()).expect("builds");
    assert_eq!(req.url, "https://ntfy.example/upSYNTH?up=1");
    for h in [
        ("content-encoding", "aes128gcm"),
        ("urgency", "high"),
        ("ttl", "86400"),
    ] {
        assert!(req.headers.contains(&(h.0.into(), h.1.into())), "{h:?}");
    }
    let plain = alert().json().to_string();
    assert!(
        !req.body.windows(plain.len()).any(|w| w == plain.as_bytes()),
        "body is not cleartext"
    );
    assert!(unifiedpush_request(&fcm_reg(), &alert()).is_err());
}

#[test]
fn the_payload_is_versioned_and_tagged() {
    let v = alert().json();
    assert_eq!(v["v"], 1);
    assert_eq!(v["kind"], "down");
    assert_eq!(v["host"], "web");
    assert_eq!(v["title"], "web is unreachable");
}

#[tokio::test]
async fn send_takes_unifiedpush_when_it_answers() {
    let sender = Scripted::new(vec![Ok(201)]);
    let hub = Hub::new(sender.clone(), Some(fcm_client()));
    hub.register(up()).await.expect("up");
    hub.register(fcm_reg()).await.expect("fcm");
    let d = hub.send(&alert()).await;
    assert_eq!(d.via, Some(Transport::UnifiedPush));
    multitop_testassert::assert_empty!(d.failed);
    assert_eq!(sender.hosts(), vec!["ntfy.example"], "FCM not touched");
}

#[tokio::test]
async fn send_falls_back_to_fcm_when_the_distributor_fails() {
    let sender = Scripted::new(vec![Err("push: connect".into()), Ok(200)]);
    let hub = Hub::new(sender.clone(), Some(fcm_client()));
    hub.register(up()).await.expect("up");
    hub.register(fcm_reg()).await.expect("fcm");
    let d = hub.send(&alert()).await;
    assert_eq!(d.via, Some(Transport::Fcm));
    assert_eq!(
        d.failed,
        vec![(Transport::UnifiedPush, "push: connect".into())]
    );
    assert_eq!(
        sender.hosts(),
        vec!["ntfy.example", "oauth2.example", "fcm.googleapis.com"]
    );
    let send = sender.seen.lock().expect("lock")[2].clone();
    let body: serde_json::Value = serde_json::from_slice(&send.body).expect("json");
    assert_eq!(body["message"]["fid"], "cSyntheticFid_1-abcdef");
    let inner: serde_json::Value =
        serde_json::from_str(body["message"]["data"]["alert"].as_str().expect("string"))
            .expect("json");
    assert_eq!(inner, alert().json(), "the same payload on both routes");
}

#[tokio::test]
async fn a_gone_endpoint_is_dropped_and_the_status_follows() {
    let sender = Scripted::new(vec![Ok(GONE), Ok(500)]);
    let hub = Hub::new(sender.clone(), Some(fcm_client()));
    hub.register(up()).await.expect("up");
    hub.register(fcm_reg()).await.expect("fcm");
    assert_eq!(hub.status().await.route, Some(Transport::UnifiedPush));
    let d = hub.send(&alert()).await;
    assert_eq!(d.via, None);
    assert_eq!(
        d.failed[0],
        (Transport::UnifiedPush, "registration gone; dropped".into())
    );
    assert_eq!(d.failed[1], (Transport::Fcm, "FCM answered 500".into()));
    let s = hub.status().await;
    assert!(!s.unifiedpush && s.fcm && s.fcm_ready);
    assert_eq!(s.route, Some(Transport::Fcm));
}

#[tokio::test]
async fn nothing_registered_sends_nothing_and_fcm_without_an_account_is_no_route() {
    let sender = Scripted::new(vec![]);
    let hub = Hub::new(sender.clone(), None);
    let d = hub.send(&alert()).await;
    assert_eq!((d.via, d.failed.len()), (None, 0));
    hub.register(fcm_reg()).await.expect("fcm");
    let s = hub.status().await;
    assert_eq!((s.fcm, s.fcm_ready, s.route), (true, false, None));
    assert!(hub.send(&alert()).await.via.is_none());
    multitop_testassert::assert_empty!(sender.hosts());
    assert!(hub
        .register(Registration::Fcm {
            token: String::new()
        })
        .await
        .is_err());
}

#[tokio::test]
async fn the_store_is_private_survives_a_restart_and_refuses_an_open_mode() {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = tempfile::tempdir().expect("dir");
    let path = dir.path().join(REGISTRATIONS_FILE);
    let sender = Scripted::new(vec![Ok(GONE)]);
    let hub = Hub::with_store(&path, sender.clone(), None).expect("empty store");
    hub.register(up()).await.expect("registers");
    let mode = std::fs::metadata(&path)
        .expect("written")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600);
    let again = Hub::with_store(&path, sender.clone(), None).expect("reloads");
    assert!(again.status().await.unifiedpush);
    // A gone endpoint is dropped from the FILE too.
    again.send(&alert()).await;
    let reread = Hub::with_store(&path, sender.clone(), None).expect("reloads");
    assert!(!reread.status().await.unifiedpush);

    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).expect("chmod");
    let err = Hub::with_store(&path, sender.clone(), None)
        .err()
        .expect("refused");
    assert!(err.contains("chmod 600"), "{err}");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).expect("chmod");
    std::fs::write(&path, "not json").expect("write");
    assert!(Hub::with_store(&path, sender, None).is_err());
}
