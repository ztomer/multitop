#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;
use tokio::sync::RwLock;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

use multitop::config::{Config, Server};
use multitop::history::History;
use multitop::server::{router, AppState, LiveState};
use multitop_agent::proc::{Proc, Usage};
use multitop_agent::render::{Snapshot, TempUnit};

fn mock_snapshot(host: &str) -> Snapshot {
    Snapshot {
        host: host.to_string(),
        agent_version: "0.45.0".to_string(),
        cpu_pct: 15.0,
        cpu_mhz: Some(2400.0),
        cores: vec![(0, 15.0, Some(45.0))],
        proc_names: vec!["init".to_string()],
        temp_unit: TempUnit::C,
        mem: Usage::new(1000, 200),
        disk: Usage::new(5000, 1000),
        rx_rate: 100.0,
        tx_rate: 200.0,
        procs: vec![Proc {
            pid: 1,
            name: "init".to_string(),
            cpu: 0.5,
            mem: 50,
        }],
    }
}

fn test_state(token: Option<String>) -> (AppState, multitop::server::SharedState) {
    let mut live = LiveState::default();
    let snap = mock_snapshot("test-host");
    let mut hist = History::default();
    hist.record(&snap);
    live.snapshots.insert("test-host".to_string(), snap);
    live.histories.insert("test-host".to_string(), hist);
    live.errors
        .insert("err-host".to_string(), "ssh connect error".to_string());

    let shared = Arc::new(RwLock::new(live));
    let cfg = Config {
        servers: vec![Server {
            host: "test-host".to_string(),
            port: 22,
            user: "root".to_string(),
            upgrade_cmd: None,
            custom_command: None,
        }],
        theme: None,
        upgrade_history_lines: 5000,
        history_lines_raised_from: None,
        banner_style: multitop::layout::BannerStyle::default(),
        plaintext_passwords: vec![],
        alert_cpu: Some(80),
        alert_mem: Some(85),
        alert_disk: Some(90),
        alerts: vec![],
    };

    let app_state = AppState {
        live: shared.clone(),
        token,
        servers: cfg.servers.clone(),
        config: cfg,
    };
    (app_state, shared)
}

#[tokio::test]
async fn test_index_page() {
    let (state, _) = test_state(Some("secret123".to_string()));
    let app = router(state);

    let req = Request::builder().uri("/").body(Body::empty()).unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let html = String::from_utf8_lossy(&bytes);
    assert!(html.contains("multitop --serve"));
}

#[tokio::test]
async fn test_auth_rejection() {
    let (state, _) = test_state(Some("valid-tok".to_string()));
    let app = router(state);

    let req = Request::builder()
        .uri("/api/hosts")
        .header("authorization", "Bearer wrong-tok")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_api_hosts_and_health() {
    let (state, _) = test_state(Some("my-token".to_string()));
    let app = router(state.clone());

    let req = Request::builder()
        .uri("/api/hosts")
        .header("authorization", "Bearer my-token")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body[0]["host"], "test-host");
    assert_eq!(body[0]["has_snapshot"], true);

    let app2 = router(state);
    let req2 = Request::builder()
        .uri("/api/health")
        .header("authorization", "my-token")
        .body(Body::empty())
        .unwrap();
    let resp2 = app2.oneshot(req2).await.unwrap();
    assert_eq!(resp2.status(), StatusCode::OK);
    let bytes2 = resp2.into_body().collect().await.unwrap().to_bytes();
    let body2: serde_json::Value = serde_json::from_slice(&bytes2).unwrap();
    assert_eq!(body2["total"], 1);
    assert_eq!(body2["breaching"], 0);
    assert_eq!(body2["healthy"], 1);
}

#[tokio::test]
async fn test_api_snapshot_and_history() {
    let (state, _) = test_state(None);
    let app = router(state.clone());

    let req = Request::builder()
        .uri("/api/snapshot/test-host")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let app_err = router(state.clone());
    let req_err = Request::builder()
        .uri("/api/snapshot/err-host")
        .body(Body::empty())
        .unwrap();
    let resp_err = app_err.oneshot(req_err).await.unwrap();
    assert_eq!(resp_err.status(), StatusCode::SERVICE_UNAVAILABLE);

    let app_missing = router(state.clone());
    let req_missing = Request::builder()
        .uri("/api/snapshot/missing-host")
        .body(Body::empty())
        .unwrap();
    let resp_missing = app_missing.oneshot(req_missing).await.unwrap();
    assert_eq!(resp_missing.status(), StatusCode::NOT_FOUND);

    let app_hist = router(state.clone());
    let req_hist = Request::builder()
        .uri("/api/history/test-host")
        .body(Body::empty())
        .unwrap();
    let resp_hist = app_hist.oneshot(req_hist).await.unwrap();
    assert_eq!(resp_hist.status(), StatusCode::OK);

    let app_mtop = router(state);
    let req_mtop = Request::builder()
        .uri("/api/mtop")
        .body(Body::empty())
        .unwrap();
    let resp_mtop = app_mtop.oneshot(req_mtop).await.unwrap();
    assert_eq!(resp_mtop.status(), StatusCode::OK);
}
