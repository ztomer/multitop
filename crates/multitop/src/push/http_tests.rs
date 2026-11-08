use super::*;

#[test]
fn target_takes_https_anywhere_and_plain_http_only_to_loopback() {
    let (_, host, port, tls) = target("https://ntfy.example/upAbc?up=1").expect("https");
    assert_eq!((host.as_str(), port, tls), ("ntfy.example", 443, true));
    let (_, _, port, tls) = target("http://127.0.0.1:8123/x").expect("loopback");
    assert_eq!((port, tls), (8123, false));
    assert!(target("http://localhost/x").is_ok());
    assert!(target("http://[::1]:9/x").is_ok());
    assert!(
        target("http://192.168.1.5/x").is_err(),
        "cleartext off loopback"
    );
    assert!(target("ftp://example/x").is_err());
    assert!(target("not a url").is_err());
    assert!(target("/relative").is_err());
}

#[test]
fn request_debug_names_the_host_and_never_the_path_or_headers() {
    let req = Request {
        url: "https://ntfy.example/upSECRETTOPIC?up=1".into(),
        headers: vec![("authorization".into(), "Bearer SECRETTOKEN".into())],
        body: vec![1, 2, 3],
    };
    let shown = format!("{req:?}");
    assert!(shown.contains("ntfy.example"), "{shown}");
    assert!(!shown.contains("SECRET"), "{shown}");
}

#[test]
fn response_ok_is_2xx_only() {
    let r = |status| Response {
        status,
        body: vec![],
    };
    assert!(r(200).ok());
    assert!(r(201).ok());
    assert!(!r(199).ok());
    assert!(!r(300).ok());
    assert!(!r(410).ok());
}

#[tokio::test]
async fn hyper_sender_posts_the_bytes_and_headers_and_reads_the_answer() {
    use axum::{body::Bytes as AxBytes, http::HeaderMap, routing::post, Router};
    let app = Router::new().route(
        "/push/{topic}",
        post(|headers: HeaderMap, body: AxBytes| async move {
            let enc = headers
                .get("content-encoding")
                .and_then(|v| v.to_str().ok())
                .unwrap_or_default()
                .to_string();
            (
                axum::http::StatusCode::CREATED,
                format!("{enc}:{}", body.len()),
            )
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move { axum::serve(listener, app).await });

    let sender = HyperSender::new().expect("client");
    let got = sender
        .post(Request {
            url: format!("http://{addr}/push/abc?up=1"),
            headers: vec![("content-encoding".into(), "aes128gcm".into())],
            body: vec![0u8; 100],
        })
        .await
        .expect("delivered");
    assert_eq!(got.status, 201);
    assert_eq!(got.body, b"aes128gcm:100");
    assert!(got.ok());
}

#[tokio::test]
async fn hyper_sender_verifies_tls_and_reports_a_refused_connection() {
    // A listener that is not TLS: the handshake must fail, not fall back.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        while let Ok((mut s, _)) = listener.accept().await {
            let _ = tokio::io::AsyncWriteExt::write_all(&mut s, b"HTTP/1.1 200 OK\r\n\r\n").await;
        }
    });
    let sender = HyperSender::new().expect("client");
    let req = |url: String| Request {
        url,
        headers: vec![],
        body: vec![],
    };
    let err = sender
        .post(req(format!("https://localhost:{}/x", addr.port())))
        .await
        .expect_err("not TLS");
    assert!(err.contains("TLS"), "{err}");

    // Nothing listening.
    let closed = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let port = closed.local_addr().expect("addr").port();
    drop(closed);
    let err = sender
        .post(req(format!("http://127.0.0.1:{port}/x")))
        .await
        .expect_err("refused");
    assert!(err.contains("connect"), "{err}");
    let err = sender
        .post(req("http://10.0.0.1/x".into()))
        .await
        .expect_err("cleartext off loopback");
    assert!(err.contains("loopback"), "{err}");
}
