// A test crate, said where clippy reads it (see server_test.rs).
#![cfg(test)]

//! A2-TLS end to end: `--serve`'s own listener, a real TLS handshake, and a
//! client that trusts exactly one certificate by its SHA-256 -- the rule the
//! Android client enforces (`PinnedTls.kt`). Proves the server half answers TLS
//! with the pinned identity, that a client pinned to any other certificate is
//! refused, and that the TLS port gives a plaintext request nothing.

use std::sync::Arc;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpStream;
use tokio::sync::RwLock;

use multitop::config::{Config, Server};
use multitop::serve_tls::{fingerprint, generate, Identity};
use multitop::server::{serve_on, AppState, LiveState};

const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn state() -> AppState {
    let cfg = Config {
        servers: vec![Server {
            host: "test-host".into(),
            port: 22,
            user: String::new(),
            upgrade_cmd: None,
            custom_command: None,
            mcp: None,
        }],
        theme: None,
        upgrade_history_lines: 5000,
        history_lines_raised_from: None,
        banner_style: multitop::layout::BannerStyle::default(),
        plaintext_passwords: vec![],
        alert_cpu: None,
        alert_mem: None,
        alert_disk: None,
        alerts: vec![],
    };
    AppState {
        live: Arc::new(RwLock::new(LiveState::default())),
        token: TOKEN.into(),
        servers: cfg.servers.clone(),
        config: cfg,
    }
}

/// The client's two layers in one: the handshake signature is checked by the
/// provider, and the leaf must BE the pinned certificate by SHA-256.
#[derive(Debug)]
struct Pinned {
    sha256: String,
    provider: Arc<rustls::crypto::CryptoProvider>,
}

impl ServerCertVerifier for Pinned {
    fn verify_server_cert(
        &self,
        leaf: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        if fingerprint(leaf) == self.sha256 {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::General(
                "leaf is not the pinned certificate".into(),
            ))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// Start `--serve`'s server on loopback port 0, TLS under `identity` when given.
async fn start(identity: Option<&Identity>) -> std::net::SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let tls = identity.map(|id| id.server_config().unwrap());
    tokio::spawn(serve_on(listener, state(), tls));
    addr
}

const REQUEST: &str = "GET /api/hosts HTTP/1.1\r\nHost: multitop\r\n\
     Authorization: Bearer 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef\r\n\
     Connection: close\r\n\r\n";

async fn get_over_tls(addr: std::net::SocketAddr, pin: &str) -> std::io::Result<String> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = rustls::ClientConfig::builder_with_provider(Arc::clone(&provider))
        .with_safe_default_protocol_versions()
        .unwrap()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(Pinned {
            sha256: pin.into(),
            provider,
        }))
        .with_no_client_auth();
    let connector = tokio_rustls::TlsConnector::from(Arc::new(config));
    let tcp = TcpStream::connect(addr).await?;
    let mut tls = connector
        .connect(ServerName::try_from("multitop").unwrap(), tcp)
        .await?;
    tls.write_all(REQUEST.as_bytes()).await?;
    let mut out = String::new();
    tls.read_to_string(&mut out).await?;
    Ok(out)
}

#[tokio::test]
async fn a_client_pinned_to_the_served_certificate_gets_the_api() {
    let id = generate().unwrap();
    let addr = start(Some(&id)).await;
    let reply = get_over_tls(addr, &id.fingerprint()).await.unwrap();
    assert!(reply.starts_with("HTTP/1.1 200"), "{reply}");
    assert!(reply.contains("test-host"), "{reply}");
}

#[tokio::test]
async fn a_client_pinned_to_another_certificate_is_refused() {
    let served = generate().unwrap();
    let other = generate().unwrap();
    let addr = start(Some(&served)).await;
    let Err(e) = get_over_tls(addr, &other.fingerprint()).await else {
        panic!("a client pinned to another certificate got an answer");
    };
    assert!(e.to_string().contains("pinned"), "{e}");
    // And the server is still up for the right client.
    assert!(get_over_tls(addr, &served.fingerprint()).await.is_ok());
}

/// Plaintext to the TLS port and the bytes that come back, if any.
async fn plaintext(addr: std::net::SocketAddr) -> String {
    let mut tcp = TcpStream::connect(addr).await.unwrap();
    let _ = tcp.write_all(REQUEST.as_bytes()).await;
    let mut out = Vec::new();
    let _ =
        tokio::time::timeout(std::time::Duration::from_secs(5), tcp.read_to_end(&mut out)).await;
    String::from_utf8_lossy(&out).into_owned()
}

#[tokio::test]
async fn the_tls_port_gives_a_plaintext_request_nothing() {
    // No HTTP answer at all -- not a redirect, not a 400 with a body: the token in
    // that request has already crossed the wire in the clear, and nothing the
    // server says back should look like it was accepted.
    let id = generate().unwrap();
    let addr = start(Some(&id)).await;
    let reply = plaintext(addr).await;
    assert!(!reply.contains("HTTP/"), "{reply:?}");
}

#[tokio::test]
async fn a_plain_server_answers_plain_http() {
    // The loopback path `needs_tls` keeps plain: the same listener without TLS.
    let addr = start(None).await;
    let reply = plaintext(addr).await;
    assert!(reply.starts_with("HTTP/1.1 200"), "{reply}");
}

#[tokio::test]
async fn a_silent_peer_does_not_hold_the_listener() {
    // Handshakes run per connection: a peer that connects and never speaks must
    // not delay the next client's handshake.
    let id = generate().unwrap();
    let addr = start(Some(&id)).await;
    let _silent = TcpStream::connect(addr).await.unwrap();
    let reply = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        get_over_tls(addr, &id.fingerprint()),
    )
    .await
    .expect("the second client waited on the silent one")
    .unwrap();
    assert!(reply.starts_with("HTTP/1.1 200"), "{reply}");
}
