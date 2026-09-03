//! `multitop --serve` — headless HTTP companion reusing the `MTOP` pipeline.
//!
//! Reuses `b"MTOP"` decoder (`multitop_agent::proto::decode_packet`) as JSON,
//! no new wire. New auth surface: `Hello` already validates `proto_version`,
//! `Token` is `Authorization: Bearer <token>` generated at startup (or
//! `--serve-token` supplied). Same `ssh` + `agent` + `history` as the TUI.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use axum::{
    extract::{Path as AxPath, State},
    http::{HeaderMap, StatusCode},
    response::{Html, IntoResponse, Json},
    routing::get,
    Router,
};
use serde::Serialize;
use tokio::sync::RwLock;

use crate::config::Server;
use crate::health;
use crate::history::History;
use multitop_agent::proto::Payload;
use multitop_agent::render::Snapshot;

/// Shared live state, updated by the same per-host monitor tasks as the TUI.
#[derive(Default, Clone)]
pub struct LiveState {
    pub snapshots: HashMap<String, Snapshot>,
    pub histories: HashMap<String, History>,
    pub errors: HashMap<String, String>,
}

pub type SharedState = Arc<RwLock<LiveState>>;

#[derive(Clone)]
pub struct AppState {
    pub live: SharedState,
    pub token: Option<String>,
    pub servers: Vec<Server>,
    pub config: crate::config::Config,
}

fn check_auth(headers: &HeaderMap, token: Option<&str>) -> Result<(), StatusCode> {
    let Some(expected) = token else {
        return Ok(());
    };
    let Some(auth) = headers.get("authorization") else {
        return Err(StatusCode::UNAUTHORIZED);
    };
    let Ok(val) = auth.to_str() else {
        return Err(StatusCode::UNAUTHORIZED);
    };
    // Accept `Bearer <token>` or raw token for curl convenience.
    let got = val.strip_prefix("Bearer ").unwrap_or(val).trim();
    if got == expected {
        Ok(())
    } else {
        Err(StatusCode::UNAUTHORIZED)
    }
}

#[derive(Serialize)]
struct HostInfo {
    host: String,
    port: u16,
    user: String,
    has_snapshot: bool,
    health: u8,
    reachable: bool,
}

async fn index() -> Html<&'static str> {
    Html(
        r#"<!doctype html><html><head><meta charset="utf-8"><title>multitop</title>
<style>body{font-family:monospace;background:#1a1b26;color:#c0caf5;padding:2rem}a{color:#7aa2f7}</style>
</head><body><h1>multitop --serve</h1>
<p>JSON: <a href="/api/hosts">/api/hosts</a> <a href="/api/health">/api/health</a></p>
<p>Per-host: <code>/api/snapshot/:host</code> <code>/api/history/:host</code></p>
<p>Token: <code>Authorization: Bearer &lt;token&gt;</code> if --serve-token was set (printed on startup)</p>
</body></html>"#,
    )
}

async fn api_hosts(State(state): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    if let Err(code) = check_auth(&headers, state.token.as_deref()) {
        return (code, Json(serde_json::json!({"error":"unauthorized"}))).into_response();
    }
    let live = state.live.read().await;
    let hosts: Vec<HostInfo> = state
        .servers
        .iter()
        .map(|s| {
            let _key = format!(
                "{}@{}:{}",
                if s.user.is_empty() {
                    "default"
                } else {
                    &s.user
                },
                s.host,
                s.port
            );
            // snapshots are keyed by snap.host (e.g. "beelink (192.168.0.33)"), but we also store by server.host
            // Check both: first by server.host, then by any snap.host containing server.host
            let snap = live.snapshots.get(&s.host).or_else(|| {
                live.snapshots
                    .iter()
                    .find(|(k, _)| k.contains(&s.host))
                    .map(|(_, v)| v)
            });
            let health = snap.map_or(100, |sn| health::health(sn, &state.config));
            HostInfo {
                host: s.host.clone(),
                port: s.port,
                user: s.user.clone(),
                has_snapshot: snap.is_some(),
                health,
                reachable: snap.is_some() || !live.errors.contains_key(&s.host),
            }
        })
        .collect();
    (StatusCode::OK, Json(serde_json::json!(hosts))).into_response()
}

async fn api_health(State(state): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    if let Err(code) = check_auth(&headers, state.token.as_deref()) {
        return (code, Json(serde_json::json!({"error":"unauthorized"}))).into_response();
    }
    let (total, breaching) = {
        let snapshots = {
            let live = state.live.read().await;
            live.snapshots.clone()
        };
        let mut total = 0;
        let mut breaching = 0;
        for s in &state.servers {
            let snap = snapshots.get(&s.host).or_else(|| {
                snapshots
                    .iter()
                    .find(|(k, _)| k.contains(&s.host))
                    .map(|(_, v)| v)
            });
            total += 1;
            if let Some(sn) = snap {
                if health::is_breaching(sn, &state.config) {
                    breaching += 1;
                }
            }
        }
        (total, breaching)
    };
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "total": total,
            "breaching": breaching,
            "healthy": total - breaching,
        })),
    )
        .into_response()
}

async fn api_snapshot(
    State(state): State<AppState>,
    AxPath(host): AxPath<String>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(code) = check_auth(&headers, state.token.as_deref()) {
        return (code, Json(serde_json::json!({"error":"unauthorized"}))).into_response();
    }
    let (resp, err_opt) = {
        let live = state.live.read().await;
        live.snapshots
            .get(&host)
            .or_else(|| {
                live.snapshots
                    .iter()
                    .find(|(k, _)| k.contains(&host))
                    .map(|(_, v)| v)
            })
            .map_or_else(
                || (None, live.errors.get(&host).cloned()),
                |snap| {
                    let v = serde_json::json!({
                        "host": snap.host,
                        "agent_version": snap.agent_version,
                        "cpu_pct": snap.cpu_pct,
                        "cpu_mhz": snap.cpu_mhz,
                        "cores": snap.cores.iter().map(|(idx,cpu,temp)| serde_json::json!({"idx": idx, "cpu": cpu, "temp": temp})).collect::<Vec<_>>(),
                        "mem": {"total": snap.mem.total, "used": snap.mem.used, "pct": snap.mem.pct},
                        "disk": {"total": snap.disk.total, "used": snap.disk.used, "pct": snap.disk.pct},
                        "rx_rate": snap.rx_rate,
                        "tx_rate": snap.tx_rate,
                        "procs": snap.procs.iter().map(|p| serde_json::json!({"pid": p.pid, "name": p.name, "cpu": p.cpu, "mem": p.mem})).collect::<Vec<_>>(),
                    });
                    (Some(v), None)
                },
            )
    };
    if let Some(v) = resp {
        return (StatusCode::OK, Json(v)).into_response();
    }
    if let Some(err) = err_opt {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error": err})),
        )
            .into_response();
    }
    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({"error": "no snapshot yet"})),
    )
        .into_response()
}

async fn api_history(
    State(state): State<AppState>,
    AxPath(host): AxPath<String>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Err(code) = check_auth(&headers, state.token.as_deref()) {
        return (code, Json(serde_json::json!({"error":"unauthorized"}))).into_response();
    }
    let entry = {
        let live = state.live.read().await;
        live.histories
            .get(&host)
            .or_else(|| {
                live.histories
                    .iter()
                    .find(|(k, _)| k.contains(&host))
                    .map(|(_, v)| v)
            })
            .cloned()
    };
    if let Some(h) = entry {
        return (StatusCode::OK, Json(h)).into_response();
    }
    (
        StatusCode::NOT_FOUND,
        Json(serde_json::json!({"error": "no history yet"})),
    )
        .into_response()
}

async fn api_mtop_raw(State(state): State<AppState>, headers: HeaderMap) -> impl IntoResponse {
    if let Err(code) = check_auth(&headers, state.token.as_deref()) {
        return (code, Json(serde_json::json!({"error":"unauthorized"}))).into_response();
    }
    let snapshots: Vec<_> = {
        let live = state.live.read().await;
        live.snapshots.values().cloned().collect()
    };
    // Reuse MTOP decoder: re-encode latest snapshots as MTOP then decode as JSON is already done.
    // This endpoint just proves the raw path is reachable; it returns the JSON form of all payloads.
    let payloads: Vec<serde_json::Value> = snapshots
        .into_iter()
        .map(|snap| {
            let payload = Payload::Monitor(snap);
            let bytes = multitop_agent::proto::encode_packet(&payload);
            // Decode to prove b"MTOP" round-trip, then serialize
            if let Some(Payload::Monitor(decoded)) = multitop_agent::proto::decode_packet(&bytes) {
                serde_json::json!({"host": decoded.host, "cpu_pct": decoded.cpu_pct, "mem_pct": decoded.mem.pct})
            } else {
                serde_json::json!({"error": "decode failed"})
            }
        })
        .collect();
    (StatusCode::OK, Json(payloads)).into_response()
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/api/hosts", get(api_hosts))
        .route("/api/health", get(api_health))
        .route("/api/snapshot/:host", get(api_snapshot))
        .route("/api/history/:host", get(api_history))
        .route("/api/mtop", get(api_mtop_raw))
        .with_state(state)
}

/// Spawn one monitor task per server that updates `live` exactly like the TUI's
/// `spawn_monitor` but without any `ratatui` rendering. Reuses `ssh::spawn_agent`
/// + `stream::connect` + `Payload::Hello` validation + `History::record`.
pub fn spawn_collectors(servers: Vec<Server>, live: &SharedState, sort: multitop_agent::SortBy) {
    use crate::ssh::Mode;
    use crate::stream::{connect, next_packet};

    for server in servers {
        let live = live.clone();
        let host_key = server.host.clone();
        tokio::spawn(async move {
            let mut failures: usize = 0;
            loop {
                let notify = |_: String| {};
                let outcome = match connect(&server, Mode::Monitor, sort, notify).await {
                    Ok(mut stream) => {
                        let mut errbuf = Vec::new();
                        let mut delivered = false;
                        let mut hello_seen = false;
                        let local_version = env!("CARGO_PKG_VERSION");
                        let mut mismatched = false;
                        while let Ok(Some(payload)) = next_packet(&mut stream, &mut errbuf).await {
                            if let Payload::Hello(hello) = &payload {
                                if hello_seen {
                                    break;
                                }
                                hello_seen = true;
                                if !hello.is_valid() || hello.needs_replacement(local_version) {
                                    mismatched = true;
                                    break;
                                }
                                continue;
                            }
                            if let Payload::Monitor(snap) = payload {
                                // Validate snapshot version fallback (old agents without Hello)
                                if !hello_seen
                                    && !snap.agent_version.is_empty()
                                    && snap.agent_version != local_version
                                {
                                    // Only treat as mismatch if Hello was missing; otherwise Hello already decided.
                                    // Old agents: snap version is the only signal.
                                    let dummy_hello = multitop_agent::proto::Hello {
                                        agent_version: snap.agent_version.clone(),
                                        proto_version: multitop_agent::proto::PROTO_VERSION,
                                        min_proto_version: multitop_agent::proto::PROTO_MIN_VERSION,
                                    };
                                    if dummy_hello.needs_replacement(local_version) {
                                        mismatched = true;
                                        break;
                                    }
                                }
                                {
                                    let mut w = live.write().await;
                                    w.snapshots.insert(host_key.clone(), snap.clone());
                                    // Also insert under snap.host for substring matching
                                    w.snapshots.insert(snap.host.clone(), snap.clone());
                                    {
                                        let hist = w.histories.entry(host_key.clone()).or_default();
                                        hist.record(&snap);
                                    }
                                    let hist_cloned =
                                        w.histories.get(&host_key).cloned().unwrap_or_default();
                                    w.histories.insert(snap.host.clone(), hist_cloned);
                                    w.errors.remove(&host_key);
                                }
                                delivered = true;
                            }
                        }
                        if mismatched {
                            let mut w = live.write().await;
                            w.errors.insert(
                                host_key.clone(),
                                "agent version mismatch — rebuild with ./build.sh".into(),
                            );
                            crate::run::SessionOutcome::NoData
                        } else if delivered {
                            crate::run::SessionOutcome::Delivered
                        } else {
                            let mut w = live.write().await;
                            let detail = errbuf
                                .last()
                                .cloned()
                                .unwrap_or_else(|| "Connection closed".into());
                            w.errors.insert(host_key.clone(), detail);
                            crate::run::SessionOutcome::NoData
                        }
                    }
                    Err(e) => {
                        let mut w = live.write().await;
                        w.errors.insert(host_key.clone(), e);
                        crate::run::SessionOutcome::NeverConnected
                    }
                };
                let wait = crate::run::reconnect_wait(outcome, &mut failures);
                tokio::time::sleep(std::time::Duration::from_secs(wait)).await;
            }
        });
    }
}

/// Start the HTTP server.
///
/// # Errors
/// Returns an error string if binding the TCP listener or serving requests fails.
pub async fn serve(addr: SocketAddr, state: AppState) -> Result<(), String> {
    let app = router(state);
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| format!("bind {addr}: {e}"))?;
    axum::serve(listener, app)
        .await
        .map_err(|e| format!("serve {addr}: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
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
}
