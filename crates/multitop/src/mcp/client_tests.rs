use std::time::Duration;

use serde_json::{json, Value};

use super::client::{
    Client, Error, META_CLIENT_CAPABILITIES, META_PROTOCOL_VERSION, PROTOCOL_VERSION,
};
use super::fake::{discover, serve, tools, Reply};

const T: Duration = Duration::from_secs(2);

fn fleet(method: &str, params: &Value) -> Reply {
    match method {
        "server/discover" => Reply::Result(discover(&["2025-11-25", PROTOCOL_VERSION])),
        // Two pages: the client must follow the cursor.
        "tools/list" if params.get("cursor").is_none() => {
            let mut page = tools(&["cron.status", "health.run"]);
            page["nextCursor"] = json!("p2");
            Reply::Result(page)
        }
        "tools/list" => Reply::Result(tools(&["alerts.since"])),
        "tools/call" => match params["name"].as_str() {
            Some("cron.status") => {
                Reply::Result(json!({"structuredContent": {"jobs": []}, "content": []}))
            }
            Some("broken") => Reply::Result(
                json!({"isError": true, "content": [{"type": "text", "text": "no such unit"}]}),
            ),
            Some("untyped") => Reply::Result(json!({"content": [{"type": "text", "text": "hi"}]})),
            Some("quiet") => Reply::Silent,
            Some("gone") => Reply::HangUp,
            _ => Reply::Error(-32602, "unknown tool"),
        },
        "resources/read" => match params["uri"].as_str() {
            Some("health://latest") => {
                Reply::Result(json!({"contents": [{"text": "{\"checks\": []}"}]}))
            }
            _ => Reply::Result(json!({"contents": [{"text": "not json"}]})),
        },
        _ => Reply::Error(-32601, "method not found"),
    }
}

#[tokio::test]
async fn discovers_lists_every_page_and_sends_the_version_on_every_request() {
    let (r, w, seen) = serve(fleet);
    let mut c = Client::connect(r, w, T).await.unwrap();
    assert_eq!(c.server, ("mcp_host".to_string(), "1.0.0".to_string()));
    assert_eq!(c.tools, ["cron.status", "health.run", "alerts.since"]);
    assert!(c.has_tool("alerts.since") && !c.has_tool("docker.containers"));
    assert_eq!(
        c.call("cron.status", json!({})).await.unwrap(),
        json!({"jobs": []})
    );
    assert_eq!(
        c.read("health://latest").await.unwrap(),
        json!({"checks": []})
    );
    let seen = seen.lock().unwrap();
    let methods: Vec<&str> = seen.iter().map(|r| r["method"].as_str().unwrap()).collect();
    assert_eq!(
        methods,
        [
            "server/discover",
            "tools/list",
            "tools/list",
            "tools/call",
            "resources/read"
        ]
    );
    for r in seen.iter() {
        assert_eq!(
            r["params"]["_meta"][META_PROTOCOL_VERSION], PROTOCOL_VERSION,
            "{r}"
        );
        assert_eq!(
            r["params"]["_meta"][META_CLIENT_CAPABILITIES],
            json!({}),
            "{r}"
        );
    }
}

#[tokio::test]
async fn an_older_server_is_refused_with_what_it_offers() {
    let (r, w, _) = serve(|m, _| match m {
        "server/discover" => Reply::Result(discover(&["2025-06-18", "2025-11-25"])),
        _ => Reply::Error(-32601, "no"),
    });
    let e = Client::connect(r, w, T).await.err().unwrap();
    assert_eq!(
        e,
        Error::Unsupported(vec!["2025-06-18".into(), "2025-11-25".into()])
    );
    assert!(
        e.to_string()
            .contains("offers protocol 2025-06-18, 2025-11-25"),
        "{e}"
    );
    // A server with no server/discover at all is a JSON-RPC error, stated.
    let (r, w, _) = serve(|_, _| Reply::Error(-32601, "method not found"));
    let e = Client::connect(r, w, T).await.err().unwrap();
    assert_eq!(e.to_string(), "method not found (-32601)");
}

#[tokio::test]
async fn each_failure_is_named_never_a_hang() {
    let (r, w, _) = serve(fleet);
    let mut c = Client::connect(r, w, Duration::from_millis(200))
        .await
        .unwrap();
    assert_eq!(
        c.call("broken", json!({})).await,
        Err(Error::Tool("broken: no such unit".into()))
    );
    assert!(matches!(
        c.call("untyped", json!({})).await,
        Err(Error::Protocol(_))
    ));
    assert_eq!(
        c.call("nope", json!({})).await,
        Err(Error::Rpc {
            code: -32602,
            message: "unknown tool".into()
        })
    );
    assert!(matches!(c.read("x://y").await, Err(Error::Protocol(m)) if m.starts_with("x://y")));
    assert!(
        c.broken().is_none(),
        "a tool's or the server's error leaves the session usable"
    );
    let e = c.call("quiet", json!({})).await.err().unwrap();
    assert!(matches!(e, Error::Timeout { .. }), "{e:?}");
    assert_eq!(e.to_string(), "tools/call: no answer in 200ms");
    // After a timeout a late reply could be taken for the next request's:
    // the session is done, and says so without asking again.
    assert_eq!(c.broken(), Some(&e));
    assert_eq!(c.call("cron.status", json!({})).await, Err(e));
    let (r, w, _) = serve(fleet);
    let mut c = Client::connect(r, w, T).await.unwrap();
    let e = c.call("gone", json!({})).await.err().unwrap();
    assert!(matches!(e, Error::Closed(_)), "{e:?}");
    assert!(
        e.to_string().starts_with("mcp_host closed the session"),
        "{e}"
    );
}
