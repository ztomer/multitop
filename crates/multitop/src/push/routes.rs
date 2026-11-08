//! The paired API's push routes, behind the same bearer token as every other.
//!
//! * `GET  /api/push`          -- what is registered and which route an alert takes.
//! * `POST /api/push/register` -- the phone's endpoint or token ([`super::Registration`]).
//! * `POST /api/push/test`     -- send one test alert now and say which route took it.

use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};

use super::{Alert, Kind, Registration};
use crate::server::AppState;

pub async fn status(State(state): State<AppState>) -> impl IntoResponse {
    Json(state.push.status().await)
}

pub async fn register(
    State(state): State<AppState>,
    body: Result<Json<Registration>, axum::extract::rejection::JsonRejection>,
) -> impl IntoResponse {
    let Ok(Json(reg)) = body else {
        return (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": "not a push registration"})),
        );
    };
    let transport = reg.transport();
    match state.push.register(reg).await {
        Ok(()) => (
            StatusCode::OK,
            Json(serde_json::json!({"registered": transport.name()})),
        ),
        Err(why) => (
            StatusCode::BAD_REQUEST,
            Json(serde_json::json!({"error": why})),
        ),
    }
}

pub async fn test(State(state): State<AppState>) -> impl IntoResponse {
    let alert = Alert::new(
        Kind::Test,
        "",
        "multitop test alert".into(),
        "Push reaches this phone.".into(),
    );
    let delivery = state.push.send(&alert).await;
    let code = if delivery.via.is_some() {
        StatusCode::OK
    } else if delivery.failed.is_empty() {
        // Nothing registered: not a server fault, a setup step.
        StatusCode::CONFLICT
    } else {
        StatusCode::BAD_GATEWAY
    };
    (code, Json(delivery))
}

#[cfg(test)]
#[path = "routes_tests.rs"]
mod tests;
