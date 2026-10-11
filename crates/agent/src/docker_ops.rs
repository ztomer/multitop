//! The Ops view's container table, for `multitop-agent mcp`.
//!
//! Not the docker view's: that one samples CPU and memory of RUNNING
//! containers. The Ops view asks a different question -- is anything
//! stopped or unhealthy that should not be -- so it lists every container
//! (`all=1`), with its state, its health as Docker words it in the status,
//! and the compose project that says whether it is a service.

use serde_json::{json, Value};

use crate::docker_transport::{http_get_on, DockerEndpoint};

/// The health a status string reports: `Up 3 hours (healthy)` is
/// `healthy`, `(health: starting)` is `starting`, no parenthesis is `none`.
#[must_use]
pub fn health_of(status: &str) -> &'static str {
    if status.contains("(unhealthy)") {
        "unhealthy"
    } else if status.contains("(healthy)") {
        "healthy"
    } else if status.contains("(health: starting)") {
        "starting"
    } else {
        "none"
    }
}

/// `GET /containers/json?all=1` as the Ops view's `docker.containers`
/// answer: `{endpoint, containers: [{name, state, status, health,
/// compose_project, image}]}`.
///
/// # Errors
///
/// The body is not the daemon's container list.
pub fn parse(json_text: &str, endpoint: &DockerEndpoint) -> Result<Value, String> {
    let items: Vec<Value> =
        serde_json::from_str(json_text).map_err(|e| format!("not a container list: {e}"))?;
    let containers: Vec<Value> = items
        .iter()
        .map(|c| {
            let status = c["Status"].as_str().unwrap_or_default();
            json!({
                "name": c["Names"][0].as_str().unwrap_or_default().trim_start_matches('/'),
                "state": c["State"].as_str().unwrap_or_default(),
                "status": status,
                "health": health_of(status),
                "compose_project": c["Labels"]["com.docker.compose.project"].as_str(),
                "image": c["Image"].as_str().unwrap_or_default(),
            })
        })
        .collect();
    Ok(json!({ "endpoint": endpoint.url(), "containers": containers }))
}

/// Ask the daemon at `endpoint`.
///
/// # Errors
///
/// The daemon could not be reached or answered something else.
pub fn list(endpoint: &DockerEndpoint) -> Result<Value, String> {
    let body = http_get_on(endpoint, "/v1.41/containers/json?all=1")
        .map_err(|e| format!("docker at {}: {e}", endpoint.url()))?;
    parse(&String::from_utf8_lossy(&body), endpoint)
}

#[cfg(test)]
#[path = "docker_ops_tests.rs"]
mod tests;
