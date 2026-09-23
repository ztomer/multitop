use super::*;
use axum::body::Body;
use axum::http::Request;
use tower::ServiceExt;

fn test_state(token: Option<String>) -> AppState {
    AppState {
        live: Arc::new(RwLock::new(LiveState::default())),
        token,
        servers: vec![Server {
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
    let app = router(test_state(None));
    let req = Request::builder().uri("/").body(Body::empty()).unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn hosts_requires_token_when_set() {
    let app = router(test_state(Some("secret123".into())));
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
    let app2 = router(test_state(Some("secret123".into())));
    let resp2 = app2.oneshot(req).await.unwrap();
    assert_eq!(resp2.status(), StatusCode::OK);
}

#[tokio::test]
async fn snapshot_not_found_yet() {
    let app = router(test_state(None));
    let req = Request::builder()
        .uri("/api/snapshot/test-host")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn mtop_raw_uses_decoder() {
    let state = test_state(None);
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
        let state = test_state(Some("tok".into()));
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
