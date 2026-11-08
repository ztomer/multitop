use super::*;
use crate::push::http::{Pending, Response};
use std::sync::Mutex;

/// A synthetic service account: a throwaway RSA key generated for this test
/// (`testdata/fcm_test_key.pem`), never a Google one.
pub fn account() -> ServiceAccount {
    ServiceAccount {
        project_id: "synthetic-project".into(),
        client_email: "pusher@synthetic-project.iam.example".into(),
        private_key: include_str!("testdata/fcm_test_key.pem").into(),
        token_uri: "https://oauth2.example/token".into(),
    }
}

fn unb64(s: &str) -> Vec<u8> {
    base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(s)
        .unwrap_or_default()
}

#[test]
fn the_assertion_is_an_rs256_jwt_google_can_verify() {
    let fcm = Fcm::new(account()).expect("test key");
    let jwt = fcm.assertion(1_000).expect("signs");
    let parts: Vec<&str> = jwt.split('.').collect();
    assert_eq!(parts.len(), 3);
    let header: serde_json::Value = serde_json::from_slice(&unb64(parts[0])).expect("json");
    assert_eq!(header["alg"], "RS256");
    let claims: serde_json::Value = serde_json::from_slice(&unb64(parts[1])).expect("json");
    assert_eq!(claims["iss"], "pusher@synthetic-project.iam.example");
    assert_eq!(claims["scope"], SCOPE);
    assert_eq!(claims["aud"], "https://oauth2.example/token");
    assert_eq!(claims["iat"], 1_000);
    assert_eq!(claims["exp"], 1_000 + ASSERTION_SECS);
    // Verified with the PUBLIC half, by ring's verifier, not by re-signing.
    let public = ring::signature::UnparsedPublicKey::new(
        &ring::signature::RSA_PKCS1_2048_8192_SHA256,
        include_bytes!("testdata/fcm_test_key.pub.der").as_slice(),
    );
    let signed = format!("{}.{}", parts[0], parts[1]);
    public
        .verify(signed.as_bytes(), &unb64(parts[2]))
        .expect("the signature verifies against the public key");
    assert!(public.verify(b"tampered", &unb64(parts[2])).is_err());
}

#[test]
fn requests_go_where_google_documents_them() {
    let fcm = Fcm::new(account()).expect("test key");
    assert!(format!("{fcm:?}").contains("synthetic-project"));
    let tr = fcm.token_request(5).expect("builds");
    assert_eq!(tr.url, "https://oauth2.example/token");
    let body = String::from_utf8(tr.body).expect("utf8");
    assert!(body.starts_with(
        "grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Ajwt-bearer&assertion="
    ));
    let send = fcm.send_request("acc", "dev", &serde_json::json!({"v": 1}));
    assert_eq!(
        send.url,
        "https://fcm.googleapis.com/v1/projects/synthetic-project/messages:send"
    );
    assert!(send
        .headers
        .contains(&("authorization".into(), "Bearer acc".into())));
    let msg: serde_json::Value = serde_json::from_slice(&send.body).expect("json");
    assert_eq!(msg["message"]["android"]["priority"], "HIGH");
    assert_eq!(msg["message"]["data"]["alert"], r#"{"v":1}"#);
    assert!(
        !format!("{fcm:?}").contains("PRIVATE"),
        "Debug never prints the key"
    );
}

#[test]
fn a_bad_account_is_refused_with_a_reason() {
    let mut a = account();
    a.private_key = "not pem at all ***".into();
    assert!(Fcm::new(a).is_err());
    let mut a = account();
    // secret-ok: four bytes of base64 under an armour line, not a key
    a.private_key = "-----BEGIN PRIVATE KEY-----\nAAAA\n-----END PRIVATE KEY-----".into();
    assert!(Fcm::new(a).is_err());
    let mut a = account();
    a.token_uri = "http://oauth2.example/token".into();
    assert!(Fcm::new(a).is_err());
}

#[test]
fn load_is_none_when_absent_and_refuses_an_open_or_broken_file() {
    use std::os::unix::fs::PermissionsExt as _;
    let dir = tempfile::tempdir().expect("dir");
    let cfg = dir.path().join("config.toml");
    let path = default_path(&cfg);
    assert_eq!(
        path.file_name().and_then(|n| n.to_str()),
        Some(SERVICE_ACCOUNT_FILE)
    );
    assert!(load(&path).expect("absent is fine").is_none());
    let json = serde_json::json!({
        "type": "service_account",
        "project_id": "synthetic-project",
        "client_email": "pusher@synthetic-project.iam.example",
        "private_key": include_str!("testdata/fcm_test_key.pem"),
        "token_uri": "https://oauth2.example/token",
    });
    std::fs::write(&path, json.to_string()).expect("write");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).expect("chmod");
    assert!(load(&path).expect_err("open").contains("chmod 600"));
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).expect("chmod");
    let loaded = load(&path).expect("loads").expect("present");
    assert!(format!("{loaded:?}").contains("synthetic-project"));
    std::fs::write(&path, "{}").expect("write");
    assert!(load(&path).is_err());
    assert!(load(dir.path()).is_err(), "a directory is not a key");
}

/// Token answers 200 once; sends answer from the script.
struct Fake {
    sends: Mutex<Vec<u16>>,
    token_calls: Mutex<u32>,
    token_status: u16,
}

impl super::Sender for Fake {
    fn post(&self, req: Request) -> Pending<'_> {
        let answer = if req.url.contains("oauth2") {
            *self.token_calls.lock().expect("lock") += 1;
            Response {
                status: self.token_status,
                body: br#"{"access_token":"synthetic-access","expires_in":3600}"#.to_vec(),
            }
        } else {
            let status = self.sends.lock().expect("lock").pop().unwrap_or(200);
            Response {
                status,
                body: vec![],
            }
        };
        Box::pin(async move { Ok(answer) })
    }
}

#[tokio::test]
async fn the_access_token_is_cached_until_near_expiry_and_404_means_gone() {
    let fcm = Fcm::new(account()).expect("test key");
    let fake = Fake {
        sends: Mutex::new(vec![404, 500, 200, 200]),
        token_calls: Mutex::new(0),
        token_status: 200,
    };
    let p = serde_json::json!({});
    assert_eq!(fcm.deliver(&fake, "dev", &p, 0).await, Ok(()));
    assert_eq!(fcm.deliver(&fake, "dev", &p, 60).await, Ok(()));
    assert_eq!(*fake.token_calls.lock().expect("lock"), 1, "cached");
    assert_eq!(
        fcm.deliver(&fake, "dev", &p, ASSERTION_SECS - REFRESH_MARGIN_SECS)
            .await,
        Err(crate::push::Failure::Other("FCM answered 500".into()))
    );
    assert_eq!(
        *fake.token_calls.lock().expect("lock"),
        2,
        "refreshed inside the margin"
    );
    assert_eq!(
        fcm.deliver(&fake, "dev", &p, ASSERTION_SECS).await,
        Err(crate::push::Failure::Gone)
    );
    let refused = Fake {
        sends: Mutex::new(vec![]),
        token_calls: Mutex::new(0),
        token_status: 401,
    };
    let fresh = Fcm::new(account()).expect("test key");
    assert!(matches!(
        fresh.deliver(&refused, "dev", &p, 0).await,
        Err(crate::push::Failure::Other(m)) if m.contains("401")
    ));
}
