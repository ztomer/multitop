// A test crate, said where clippy reads it: the restriction lints
// (`unwrap_used`, `expect_used`, `panic`) are policy for production code and
// exempt for test code (clippy.toml), and an integration test is test code
// through and through -- helpers included.
#![cfg(test)]

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

fn test_state(token: &str) -> (AppState, multitop::server::SharedState) {
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
            name: None,
            host: "test-host".to_string(),
            port: 22,
            user: "root".to_string(),
            upgrade_cmd: None,
            custom_command: None,
            mcp: None,
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
        token: token.to_string(),
        servers: cfg.servers.clone(),
        config: cfg,
    };
    (app_state, shared)
}

/// The `tower` this crate declares can drive the `Router` `axum` builds.
///
/// This test exists because of a pin, and the pin was invisible until it was
/// named. `crates/multitop/Cargo.toml` declared `tower = "0.4"` while `axum
/// 0.7` already resolved `0.5`, so the tree carried two majors of one crate.
/// It compiled anyway, and the reason is worth recording rather than
/// rediscovering: `ServiceExt::oneshot` is a blanket impl over
/// `tower_service::Service`, there is exactly one `tower-service` (0.3.3) in
/// the tree, and so 0.4's extension trait was satisfying a `Router` that axum
/// had built against 0.5. Two majors, no type error, nothing to notice.
///
/// So the claim is COMPILED rather than inferred: the `oneshot` below resolves
/// through this crate's `tower` and is applied to an `axum::Router`. If the
/// tree ever carried a second `tower-service`, this stops compiling instead of
/// quietly working.
///
/// What this does NOT catch, so it is not claimed to: a re-pin of `tower`
/// itself. `tower = "0.4"` compiled and passed right here -- that is the whole
/// reason the pin was invisible. The gate that owns that case is
/// `pinned-below-graph` in the rust gate, which fails on a direct dependency
/// declared below what the graph already resolves.
///
/// It asserts a 200 as well, because a test that only proves a trait resolves
/// proves the spelling and not the behaviour.
#[tokio::test]
async fn the_tower_this_crate_declares_drives_the_router_axum_builds() {
    let (state, _) = test_state("secret123");
    let req = Request::builder().uri("/").body(Body::empty()).unwrap();
    let resp = router(state).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_index_page() {
    let (state, _) = test_state("secret123");
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
    let (state, _) = test_state("valid-tok");
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
    let (state, _) = test_state("my-token");
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
    // A TOKEN, and a header on every request below. This test passed `None` and sent
    // no `Authorization`, which only worked while `check_auth` returned `Ok(())` on a
    // missing token -- i.e. it was reading "unauthenticated is fine" as a property of
    // the API. Its actual subject is the 200/503/404 the snapshot endpoint returns per
    // host, and that needs a request that gets past auth to be about anything.
    let (state, _) = test_state("snap-tok");
    let app = router(state.clone());

    let req = Request::builder()
        .header("authorization", "Bearer snap-tok")
        .uri("/api/snapshot/test-host")
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let app_err = router(state.clone());
    let req_err = Request::builder()
        .header("authorization", "Bearer snap-tok")
        .uri("/api/snapshot/err-host")
        .body(Body::empty())
        .unwrap();
    let resp_err = app_err.oneshot(req_err).await.unwrap();
    assert_eq!(resp_err.status(), StatusCode::SERVICE_UNAVAILABLE);

    let app_missing = router(state.clone());
    let req_missing = Request::builder()
        .header("authorization", "Bearer snap-tok")
        .uri("/api/snapshot/missing-host")
        .body(Body::empty())
        .unwrap();
    let resp_missing = app_missing.oneshot(req_missing).await.unwrap();
    assert_eq!(resp_missing.status(), StatusCode::NOT_FOUND);

    let app_hist = router(state.clone());
    let req_hist = Request::builder()
        .header("authorization", "Bearer snap-tok")
        .uri("/api/history/test-host")
        .body(Body::empty())
        .unwrap();
    let resp_hist = app_hist.oneshot(req_hist).await.unwrap();
    assert_eq!(resp_hist.status(), StatusCode::OK);

    let app_mtop = router(state);
    let req_mtop = Request::builder()
        .header("authorization", "Bearer snap-tok")
        .uri("/api/mtop")
        .body(Body::empty())
        .unwrap();
    let resp_mtop = app_mtop.oneshot(req_mtop).await.unwrap();
    assert_eq!(resp_mtop.status(), StatusCode::OK);
}

/// GET `uri` with the right token and parse the JSON body.
async fn get_json(state: AppState, uri: &str) -> serde_json::Value {
    let req = Request::builder()
        .uri(uri)
        .header("authorization", "Bearer unread-tok")
        .body(Body::empty())
        .unwrap();
    let resp = router(state).oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

/// A host the server has not read yet is NOT healthy (Android A2.5, the phone's
/// first reader of these two endpoints). Before this, `/api/hosts` gave it
/// `"health": 100` -- a perfect score for nothing measured -- and `/api/health`
/// counted it in `healthy`, so a server that had read nothing at all said every
/// host was well. Unread is its own count, and its health is absent.
#[tokio::test]
async fn a_host_with_no_reading_is_never_counted_healthy() {
    let (mut state, _) = test_state("unread-tok");
    let mut unread = state.servers[0].clone();
    unread.host = "unread-host".into();
    unread.name = Some("beelink".into());
    state.servers.push(unread);

    let hosts = get_json(state.clone(), "/api/hosts").await;
    assert_eq!(hosts[0]["host"], "test-host");
    assert!(hosts[0]["health"].is_u64(), "{hosts}");
    assert_eq!(hosts[0]["breaching"], false, "{hosts}");
    assert_eq!(hosts[1]["host"], "unread-host");
    assert_eq!(hosts[1]["name"], "beelink");
    assert_eq!(hosts[1]["has_snapshot"], false);
    assert!(hosts[1]["health"].is_null(), "{hosts}");
    assert_eq!(hosts[1]["breaching"], false, "{hosts}");

    let health = get_json(state, "/api/health").await;
    assert_eq!(health["total"], 2, "{health}");
    assert_eq!(health["healthy"], 1, "{health}");
    assert_eq!(health["breaching"], 0, "{health}");
    assert_eq!(health["unread"], 1, "{health}");
}
