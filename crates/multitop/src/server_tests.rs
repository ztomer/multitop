use super::*;
use axum::body::Body;
use axum::http::Request;
use tower::ServiceExt;

fn test_state(token: &str) -> AppState {
    AppState {
        live: Arc::new(RwLock::new(LiveState::default())),
        token: token.to_string(),
        servers: vec![Server {
            name: None,
            host: "test-host".into(),
            port: 22,
            user: String::new(),
            upgrade_cmd: None,
            custom_command: None,
            mcp: None,
        }],
        config: crate::config::Config {
            servers: vec![],
            theme: None,
            upgrade_history_lines: 5000,
            history_lines_raised_from: None,
            banner_style: crate::layout::BannerStyle::default(),
            plaintext_passwords: vec![],
            alert_cpu: None,
            alert_mem: None,
            alert_disk: None,
            alerts: vec![],
        },
    }
}

#[tokio::test]
async fn index_ok_without_token() {
    let app = router(test_state(""));
    let req = Request::builder().uri("/").body(Body::empty()).unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn hosts_requires_token_when_set() {
    let app = router(test_state("secret123"));
    let req = Request::builder()
        .uri("/api/hosts")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    let req = Request::builder()
        .uri("/api/hosts")
        .header("authorization", "Bearer secret123")
        .body(Body::empty())
        .unwrap();
    let app2 = router(test_state("secret123"));
    let resp2 = app2.oneshot(req).await.unwrap();
    assert_eq!(resp2.status(), StatusCode::OK);
}

/// GET `uri` and return the status with the body as JSON (`Null` when the body
/// is not JSON -- which is what an unmatched route answers).
/// The token these tests authenticate with. One constant rather than a literal per
/// test: `test_state(None)` used to mean "no auth needed", which only worked while
/// `check_auth` returned `Ok(())` on a missing token. These tests are about handlers
/// and decoders, so they authenticate.
const TOKEN: &str = "test-token";

async fn get_json(uri: &str) -> (StatusCode, serde_json::Value) {
    let app = router(test_state(TOKEN));
    let req = Request::builder()
        .uri(uri)
        .header("authorization", format!("Bearer {TOKEN}"))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let status = resp.status();
    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null),
    )
}

/// Each parameterised route reaches ITS handler. A bare 404 cannot show that:
/// an unmatched route answers 404 too, so a route whose parameter syntax the
/// router no longer reads (axum 0.8 moved `/:host` to `/{host}`) would pass a
/// status-only check. The handler's own error body is the proof it ran.
#[tokio::test]
async fn parameterised_routes_reach_their_handlers() {
    for (uri, error) in [
        ("/api/snapshot/test-host", "no snapshot yet"),
        ("/api/history/test-host", "no history yet"),
    ] {
        let (status, body) = get_json(uri).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{uri}");
        assert_eq!(body["error"], error, "{uri} did not reach its handler");
    }
    // The control: a path no route matches is a 404 with no handler body.
    let (status, body) = get_json("/api/snapshot").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body, serde_json::Value::Null);
}

#[tokio::test]
async fn mtop_raw_uses_decoder() {
    let state = test_state(TOKEN);
    // Pre-populate a snapshot to prove MTOP encode/decode path
    {
        let mut w = state.live.write().await;
        w.snapshots.insert(
            "test-host".into(),
            Snapshot {
                host: "test-host".into(),
                agent_version: "0.44.2".into(),
                cpu_pct: 42.0,
                cpu_mhz: None,
                proc_names: vec![],
                cores: vec![],
                temp_unit: multitop_agent::render::TempUnit::C,
                mem: multitop_agent::proc::Usage::new(100, 50),
                disk: multitop_agent::proc::Usage::new(100, 20),
                rx_rate: 0.0,
                tx_rate: 0.0,
                procs: vec![],
            },
        );
    }
    let app = router(state);
    let req = Request::builder()
        .uri("/api/mtop")
        .header("authorization", format!("Bearer {TOKEN}"))
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(v.is_array());
    assert_eq!(v[0]["host"], "test-host");
    assert_eq!(v[0]["cpu_pct"], 42.0);
}

#[test]
fn hello_token_is_checked() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let state = test_state("tok");
        let app = router(state);
        let req = Request::builder()
            .uri("/api/health")
            .header("authorization", "Bearer wrong")
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    });
}

// ── P0.1: the `--serve` token ────────────────────────────────────────────────────
//
// The token is the only thing between the network and every host's metrics on the
// LAN bind the phone needs (`--serve 0.0.0.0:8080`). Three defects lived in one
// place and are pinned here together.

#[tokio::test]
async fn an_empty_token_still_refuses_an_unauthenticated_request() {
    // RED before the fix, and this is the trap it closes. `AppState::token` was
    // `Option<String>` and `check_auth` returned `Ok(())` on `None`, so "no token
    // configured" silently meant "no authentication". The binary never took that path,
    // so it read as dead code -- an open door with a test-shaped handle.
    //
    // The type now makes it unrepresentable, so what is left to pin is the VALUE
    // case: an empty token authenticates nothing. Without this arm, `AppState::token`
    // could become a `String` that an empty `Default::default()` fills, and every
    // request would fail against "" — or, worse, be handed "" as the printed token.
    let app = router(test_state(""));
    let req = Request::builder()
        .uri("/api/hosts")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    // And an EMPTY PRESENTED token is refused against a real one. Two spellings, and
    // the second is here because calibration caught its absence: a bare `Bearer` with
    // no trailing space does not strip, so `got` stays the literal `"Bearer"` and a
    // mutation that accepts an empty credential is not reached at all. A test that only
    // sends `Bearer` passes against code that would let `""` through.
    for header in ["Bearer ", ""] {
        let app = router(test_state("secret123"));
        let req = Request::builder()
            .uri("/api/hosts")
            .header("authorization", header)
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "authorization {header:?} was accepted"
        );
    }
}

// ── A1: LAN-bind hardening ───────────────────────────────────────────────────────

/// GET `uri` with `authorization` (or none) through a router holding [`TOKEN`], and
/// return the status, every response header value, and the body as text.
async fn get_raw(uri: &str, authorization: Option<&str>) -> (StatusCode, Vec<String>, String) {
    let mut req = Request::builder().uri(uri);
    if let Some(value) = authorization {
        req = req.header("authorization", value);
    }
    let resp = router(test_state(TOKEN))
        .oneshot(req.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let headers = resp
        .headers()
        .values()
        .map(|v| String::from_utf8_lossy(v.as_bytes()).into_owned())
        .collect();
    let body = axum::body::to_bytes(resp.into_body(), 1024 * 1024)
        .await
        .unwrap();
    (status, headers, String::from_utf8_lossy(&body).into_owned())
}

#[tokio::test]
async fn an_unmatched_path_is_refused_before_it_is_a_404() {
    // RED before A1: auth was a `check_auth` call at the top of each handler, so it
    // covered exactly the routes someone remembered to add it to. An unmatched path
    // reached no handler and answered 404 to anyone -- the visible half of a class
    // whose other half is the next route added without the call. The layer is
    // router-wide, fallback included; this is the probe that shows it.
    for uri in ["/api/not-a-route", "/api/snapshot", "/not-a-route"] {
        let (status, _, _) = get_raw(uri, None).await;
        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "{uri} answered unauthenticated"
        );
        let (status, _, _) = get_raw(uri, Some(&format!("Bearer {TOKEN}"))).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{uri} with the token");
    }
    // The one exemption is exactly the index.
    let (status, _, _) = get_raw("/", None).await;
    assert_eq!(status, StatusCode::OK);
}

#[tokio::test]
async fn a_refusal_echoes_nothing_the_caller_sent() {
    // A 401 that names what was presented puts a credential -- the right one with a
    // typo, or the one a client sent to the wrong server -- into every log that
    // records response bodies or headers. The refusal must be the same few bytes
    // whatever arrived. The near miss is the real token with its last byte changed:
    // the value most worth not echoing.
    let near_miss = format!("{}x", &TOKEN[..TOKEN.len() - 1]);
    let marker = "presented-credential-marker-5e1d";
    for presented in [
        format!("Bearer {marker}"),
        marker.to_string(),
        format!("Bearer {near_miss}"),
        format!("Basic {marker}"),
    ] {
        for uri in ["/api/hosts", "/api/snapshot/test-host", "/api/not-a-route"] {
            let (status, headers, body) = get_raw(uri, Some(&presented)).await;
            assert_eq!(status, StatusCode::UNAUTHORIZED, "{uri} {presented:?}");
            assert_eq!(body, r#"{"error":"unauthorized"}"#, "{uri} {presented:?}");
            for needle in [marker, near_miss.as_str(), TOKEN] {
                assert!(
                    headers.iter().all(|h| !h.contains(needle)),
                    "{uri}: a response header carries {needle:?}: {headers:?}"
                );
            }
        }
    }
}
