use crate::push::http::{Pending, Request as PushRequest, Response, Sender};
use crate::push::Hub;
use crate::server::{router, AppState, LiveState};
use axum::body::Body;
use axum::http::{Request, StatusCode};
use base64::Engine as _;
use std::sync::{Arc, Mutex};
use tower::ServiceExt;

const TOKEN: &str = "synthetic-serve-token";

#[derive(Default)]
struct Recorder(Mutex<Vec<PushRequest>>);

impl Sender for Recorder {
    fn post(&self, req: PushRequest) -> Pending<'_> {
        self.0.lock().expect("lock").push(req);
        Box::pin(async {
            Ok(Response {
                status: 201,
                body: vec![],
            })
        })
    }
}

fn state(sender: Arc<Recorder>) -> AppState {
    AppState {
        live: Arc::new(tokio::sync::RwLock::new(LiveState::default())),
        token: TOKEN.into(),
        servers: vec![],
        config: crate::config::Config {
            servers: vec![],
            theme: None,
            upgrade_history_lines: crate::config::DEFAULT_UPGRADE_HISTORY_LINES,
            history_lines_raised_from: None,
            banner_style: crate::layout::BannerStyle::default(),
            plaintext_passwords: vec![],
            alert_cpu: None,
            alert_mem: None,
            alert_disk: None,
            alerts: vec![],
        },
        push: Arc::new(Hub::new(sender, None)),
    }
}

async fn call(
    st: &AppState,
    method: &str,
    uri: &str,
    body: &str,
    auth: bool,
) -> (StatusCode, serde_json::Value) {
    let mut b = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json");
    if auth {
        b = b.header("authorization", format!("Bearer {TOKEN}"));
    }
    let resp = router(st.clone())
        .oneshot(b.body(Body::from(body.to_string())).expect("request"))
        .await
        .expect("answers");
    let status = resp.status();
    let bytes = http_body_util::BodyExt::collect(resp.into_body())
        .await
        .expect("body")
        .to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap_or_default())
}

fn up_json() -> String {
    let rng = ring::rand::SystemRandom::new();
    let key = ring::agreement::EphemeralPrivateKey::generate(&ring::agreement::ECDH_P256, &rng)
        .expect("key");
    let b = |x: &[u8]| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(x);
    serde_json::json!({
        "kind": "unifiedpush",
        "endpoint": "https://ntfy.example/upSYNTH?up=1",
        "p256dh": b(key.compute_public_key().expect("pub").as_ref()),
        "auth": b(&[3u8; 16]),
    })
    .to_string()
}

#[tokio::test]
async fn push_routes_need_the_token() {
    let st = state(Arc::default());
    for (m, uri) in [
        ("GET", "/api/push"),
        ("POST", "/api/push/register"),
        ("POST", "/api/push/test"),
    ] {
        let (code, _) = call(&st, m, uri, "{}", false).await;
        assert_eq!(code, StatusCode::UNAUTHORIZED, "{m} {uri}");
    }
}

#[tokio::test]
async fn register_then_test_delivers_over_unifiedpush() {
    let sender = Arc::new(Recorder::default());
    let st = state(sender.clone());
    let (code, body) = call(&st, "POST", "/api/push/test", "", true).await;
    assert_eq!(
        code,
        StatusCode::CONFLICT,
        "nothing registered is a setup step: {body}"
    );

    let (code, body) = call(&st, "GET", "/api/push", "", true).await;
    assert_eq!(code, StatusCode::OK);
    assert_eq!(body["route"], serde_json::Value::Null);

    let (code, body) = call(&st, "POST", "/api/push/register", &up_json(), true).await;
    assert_eq!(
        (code, body["registered"].as_str()),
        (StatusCode::OK, Some("unifiedpush"))
    );
    let (_, body) = call(&st, "GET", "/api/push", "", true).await;
    assert_eq!(body["route"], "unifiedpush");

    let (code, body) = call(&st, "POST", "/api/push/test", "", true).await;
    assert_eq!(
        (code, body["via"].as_str()),
        (StatusCode::OK, Some("unifiedpush"))
    );
    assert_eq!(sender.0.lock().expect("lock").len(), 1);
}

#[tokio::test]
async fn a_bad_registration_is_a_400_that_names_no_value() {
    let st = state(Arc::default());
    let (code, _) = call(&st, "POST", "/api/push/register", "not json", true).await;
    assert_eq!(code, StatusCode::BAD_REQUEST);
    let bad =
        r#"{"kind":"unifiedpush","endpoint":"http://10.1.1.1/SECRET","p256dh":"x","auth":"y"}"#;
    let (code, body) = call(&st, "POST", "/api/push/register", bad, true).await;
    assert_eq!(code, StatusCode::BAD_REQUEST);
    assert!(!body.to_string().contains("SECRET"), "{body}");
}

#[tokio::test]
async fn a_failed_delivery_is_a_502_with_the_reason() {
    struct Down;
    impl Sender for Down {
        fn post(&self, _: PushRequest) -> Pending<'_> {
            Box::pin(async { Err("push: connect ntfy.example: refused".to_string()) })
        }
    }
    let mut st = state(Arc::default());
    st.push = Arc::new(Hub::new(Arc::new(Down), None));
    let (code, _) = call(&st, "POST", "/api/push/register", &up_json(), true).await;
    assert_eq!(code, StatusCode::OK);
    let (code, body) = call(&st, "POST", "/api/push/test", "", true).await;
    assert_eq!(code, StatusCode::BAD_GATEWAY);
    assert_eq!(body["failed"][0][0], "unifiedpush");
}
