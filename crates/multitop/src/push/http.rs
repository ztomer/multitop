//! The push client's one impure edge: POST some bytes, read the status.
//!
//! Both transports build a [`Request`] (pure, tested on its bytes) and hand it to
//! a [`Sender`]. The real one is hyper's HTTP/1 connection over rustls with
//! Mozilla's roots; the tests' is a fake that records what it was asked to send
//! and answers from a script.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use http_body_util::{BodyExt as _, Full};
use hyper::body::Bytes;

/// How long one push may take, connect to last byte. A push that has not
/// landed by then is a failure the next route can still serve.
pub const POST_TIMEOUT: Duration = Duration::from_secs(20);
/// The most of a response body kept: enough for an error message or a token.
pub const MAX_RESPONSE: usize = 64 * 1024;
/// HTTP's success class (RFC 9110 section 15.3): what a push that landed answers.
pub const SUCCESS: std::ops::Range<u16> = 200..300;
const HTTPS_PORT: u16 = 443;
const HTTP_PORT: u16 = 80;

/// One POST. The URL and headers can carry capabilities (an endpoint is one, a
/// bearer token another), so `Debug` names only the host.
#[derive(Clone, PartialEq, Eq)]
pub struct Request {
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl std::fmt::Debug for Request {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let host = self
            .url
            .parse::<hyper::Uri>()
            .ok()
            .and_then(|u| u.host().map(str::to_string))
            .unwrap_or_default();
        f.debug_struct("Request")
            .field("host", &host)
            .field("body_len", &self.body.len())
            .finish_non_exhaustive()
    }
}

/// What came back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    pub body: Vec<u8>,
}

impl Response {
    #[must_use]
    pub fn ok(&self) -> bool {
        is_success(self.status)
    }
}

/// One definition of "it landed", for every route.
#[must_use]
pub fn is_success(status: u16) -> bool {
    SUCCESS.contains(&status)
}

/// A boxed send future: the trait stays object-safe, so the hub holds one.
pub type Pending<'a> = Pin<Box<dyn Future<Output = Result<Response, String>> + Send + 'a>>;

/// The seam: who actually puts the bytes on the wire.
pub trait Sender: Send + Sync {
    fn post(&self, req: Request) -> Pending<'_>;
}

/// The real sender: hyper over rustls, the `ring` provider and Mozilla's roots.
///
/// Plain `http` is accepted only for a loopback host, which is what a local
/// distributor and the tests use; a LAN or internet endpoint is TLS or refused.
pub struct HyperSender {
    tls: tokio_rustls::TlsConnector,
    /// The owner's private distributor and the connector that also trusts
    /// its roots - used for its origin ONLY ([`super::distributor`]).
    distributor: Option<(super::distributor::Distributor, tokio_rustls::TlsConnector)>,
}

fn connector(
    extra: &[rustls::pki_types::CertificateDer<'static>],
) -> Result<tokio_rustls::TlsConnector, String> {
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    for cert in extra {
        roots
            .add(cert.clone())
            .map_err(|e| format!("push TLS: distributor ca: {e}"))?;
    }
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| format!("push TLS: {e}"))?
        .with_root_certificates(roots)
        .with_no_client_auth();
    Ok(tokio_rustls::TlsConnector::from(Arc::new(config)))
}

impl HyperSender {
    /// # Errors
    ///
    /// When rustls cannot build a client for the `ring` provider.
    pub fn new() -> Result<Self, String> {
        Ok(Self {
            tls: connector(&[])?,
            distributor: None,
        })
    }

    /// The sender with the owner's distributor: its origin gets the bearer and
    /// is verified against Mozilla's roots plus its own; every other host is
    /// exactly as [`HyperSender::new`] treats it.
    ///
    /// # Errors
    ///
    /// When rustls cannot build a client, or refuses one of the extra roots.
    pub fn with_distributor(d: super::distributor::Distributor) -> Result<Self, String> {
        let own = connector(d.roots())?;
        Ok(Self {
            tls: connector(&[])?,
            distributor: Some((d, own)),
        })
    }
}

/// Where a URL points and whether it is TLS, refusing plain HTTP off loopback.
///
/// # Errors
///
/// An unparseable URL, a scheme other than https/http, or http to a host that
/// is not loopback.
pub fn target(url: &str) -> Result<(hyper::Uri, String, u16, bool), String> {
    let uri: hyper::Uri = url.parse().map_err(|_| "push: not a URL".to_string())?;
    let host = uri.host().ok_or("push: URL has no host")?.to_string();
    let tls = match uri.scheme_str() {
        Some("https") => true,
        Some("http") if is_loopback(&host) => false,
        Some("http") => return Err("push: plain http is accepted only to loopback".into()),
        _ => return Err("push: URL is not https".into()),
    };
    let port = uri
        .port_u16()
        .unwrap_or(if tls { HTTPS_PORT } else { HTTP_PORT });
    Ok((uri, host, port, tls))
}

fn is_loopback(host: &str) -> bool {
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    bare == "localhost"
        || bare
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

async fn exchange<S>(io: S, uri: &hyper::Uri, host: &str, req: Request) -> Result<Response, String>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let (mut send, conn) = hyper::client::conn::http1::handshake(hyper_util::rt::TokioIo::new(io))
        .await
        .map_err(|e| format!("push: handshake: {e}"))?;
    tokio::spawn(conn);
    let path = uri
        .path_and_query()
        .map_or("/", hyper::http::uri::PathAndQuery::as_str);
    let mut builder = hyper::Request::post(path).header("host", host);
    for (k, v) in &req.headers {
        builder = builder.header(k.as_str(), v.as_str());
    }
    let request = builder
        .body(Full::new(Bytes::from(req.body)))
        .map_err(|e| format!("push: request: {e}"))?;
    let response = send
        .send_request(request)
        .await
        .map_err(|e| format!("push: send: {e}"))?;
    let status = response.status().as_u16();
    let collected = http_body_util::Limited::new(response.into_body(), MAX_RESPONSE)
        .collect()
        .await
        .map_err(|e| format!("push: response: {e}"))?;
    Ok(Response {
        status,
        body: collected.to_bytes().to_vec(),
    })
}

impl HyperSender {
    async fn send(&self, req: Request) -> Result<Response, String> {
        let (req, connector) = match &self.distributor {
            Some((d, own)) if d.serves(&req.url) => (d.authorize(req), own),
            _ => (req, &self.tls),
        };
        let (uri, host, port, tls) = target(&req.url)?;
        let bare = host
            .trim_start_matches('[')
            .trim_end_matches(']')
            .to_string();
        let tcp = tokio::net::TcpStream::connect((bare.as_str(), port))
            .await
            .map_err(|e| format!("push: connect {host}: {e}"))?;
        if tls {
            let name = rustls::pki_types::ServerName::try_from(bare)
                .map_err(|_| format!("push: bad host {host}"))?;
            let stream = connector
                .connect(name, tcp)
                .await
                .map_err(|e| format!("push: TLS to {host}: {e}"))?;
            exchange(stream, &uri, &host, req).await
        } else {
            exchange(tcp, &uri, &host, req).await
        }
    }
}

impl Sender for HyperSender {
    fn post(&self, req: Request) -> Pending<'_> {
        Box::pin(async move {
            tokio::time::timeout(POST_TIMEOUT, self.send(req))
                .await
                .map_err(|_| "push: timed out".to_string())?
        })
    }
}

#[cfg(test)]
#[path = "http_tests.rs"]
mod tests;
