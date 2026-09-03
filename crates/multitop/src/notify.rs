//! Alert and upgrade notification dispatch (Phase 4 — Notify).
//!
//! Dispatches notifications to webhooks (e.g. ntfy.sh, Slack/Discord webhooks)
//! and local desktop notifications (macOS Notification Center / Linux notify-send)
//! without introducing heavy HTTP client dependencies or blocking the TUI event loop.

use crate::config::{AlertTarget, Server};

/// Format human-readable title and body for an upgrade completion.
#[must_use]
pub fn format_upgrade_notification(
    server: &Server,
    success: bool,
    duration_secs: Option<u64>,
) -> (String, String) {
    let outcome = if success { "succeeded" } else { "failed" };
    let title = format!("multitop: upgrade {outcome} on {}", server.host);
    let dur_str = duration_secs.map_or_else(String::new, |d| {
        format!(" in {}", crate::upgrade_view::fmt_duration(d))
    });
    let body = format!("Host {} upgrade {outcome}{dur_str}.", server.host);
    (title, body)
}

/// Format human-readable title and body for a resource metric breach.
#[must_use]
pub fn format_breach_notification(
    server: &Server,
    metric: &str,
    value: u8,
    threshold: u8,
) -> (String, String) {
    let title = format!("multitop: alert on {}", server.host);
    let body = format!(
        "Host {} {} breached threshold: {}% > {}%",
        server.host, metric, value, threshold
    );
    (title, body)
}

/// Asynchronously dispatch upgrade notifications to all configured targets.
pub fn dispatch_upgrade_notification(
    server: &Server,
    success: bool,
    duration_secs: Option<u64>,
    targets: &[AlertTarget],
) {
    if targets.is_empty() {
        return;
    }
    let (title, body) = format_upgrade_notification(server, success, duration_secs);
    let targets = targets.to_vec();
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        handle.spawn(async move {
            dispatch_all(&title, &body, &targets).await;
        });
    }
}

/// Asynchronously dispatch metric breach notifications to all configured targets.
pub fn dispatch_breach_notification(
    server: &Server,
    metric: &str,
    value: u8,
    threshold: u8,
    targets: &[AlertTarget],
) {
    if targets.is_empty() {
        return;
    }
    let (title, body) = format_breach_notification(server, metric, value, threshold);
    let targets = targets.to_vec();
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        handle.spawn(async move {
            dispatch_all(&title, &body, &targets).await;
        });
    }
}

async fn dispatch_all(title: &str, body: &str, targets: &[AlertTarget]) {
    for target in targets {
        if let Some(url) = target.webhook.as_deref() {
            let _ = post_webhook(url, title, body).await;
        }
        if target.desktop {
            let _ = send_desktop_notification(title, body).await;
        }
    }
}

async fn post_webhook(url: &str, title: &str, body: &str) -> std::io::Result<()> {
    // Uses curl for universal zero-dependency, non-blocking webhook delivery
    // (native on macOS and all Linux distributions).
    let mut cmd = tokio::process::Command::new("curl");
    cmd.arg("-s")
        .arg("-X")
        .arg("POST")
        .arg("-H")
        .arg(format!("Title: {title}"))
        .arg("-d")
        .arg(body)
        .arg(url)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());

    let _ = cmd.status().await;
    Ok(())
}

async fn send_desktop_notification(title: &str, body: &str) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        let script = format!(
            "display notification \"{}\" with title \"{}\"",
            body.replace('"', "\\\""),
            title.replace('"', "\\\"")
        );
        let mut cmd = tokio::process::Command::new("osascript");
        cmd.arg("-e").arg(script);
        let _ = cmd.status().await;
    }

    #[cfg(not(target_os = "macos"))]
    {
        let mut cmd = tokio::process::Command::new("notify-send");
        cmd.arg(title).arg(body);
        let _ = cmd.status().await;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_upgrade_success() {
        let server = Server {
            host: "db-prod".into(),
            ..Default::default()
        };
        let (title, body) = format_upgrade_notification(&server, true, Some(75));
        assert_eq!(title, "multitop: upgrade succeeded on db-prod");
        assert_eq!(body, "Host db-prod upgrade succeeded in 1m 15s.");
    }

    #[test]
    fn format_upgrade_failure() {
        let server = Server {
            host: "web-01".into(),
            ..Default::default()
        };
        let (title, body) = format_upgrade_notification(&server, false, None);
        assert_eq!(title, "multitop: upgrade failed on web-01");
        assert_eq!(body, "Host web-01 upgrade failed.");
    }

    #[test]
    fn format_breach() {
        let server = Server {
            host: "worker-02".into(),
            ..Default::default()
        };
        let (title, body) = format_breach_notification(&server, "CPU", 92, 80);
        assert_eq!(title, "multitop: alert on worker-02");
        assert_eq!(body, "Host worker-02 CPU breached threshold: 92% > 80%");
    }

    #[tokio::test]
    async fn dispatch_empty_targets_noop() {
        let server = Server {
            host: "dummy".into(),
            ..Default::default()
        };
        dispatch_upgrade_notification(&server, true, None, &[]);
        dispatch_breach_notification(&server, "MEM", 95, 85, &[]);
    }

    #[tokio::test]
    async fn dispatch_with_targets() {
        let server = Server {
            host: "srv-alert".into(),
            ..Default::default()
        };
        let targets = vec![
            AlertTarget {
                webhook: Some("http://127.0.0.1:9/nowhere".into()),
                desktop: false,
            },
            AlertTarget {
                webhook: None,
                desktop: true,
            },
        ];
        dispatch_upgrade_notification(&server, false, Some(10), &targets);
        dispatch_breach_notification(&server, "DISK", 99, 90, &targets);
        dispatch_all("Title", "Body", &targets).await;
    }
}
