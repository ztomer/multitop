//! A2-TLS, the server half: the `--serve` certificate, and TLS over it.
//!
//! **Why TLS, and why this shape** (docs/roadmap.md, A1's decision and owner
//! ruling D4). On a LAN bind the bearer token and every response cross a wire
//! anyone nearby can read, and Android refuses cleartext by default. There is no
//! CA and no domain, so the server makes ONE self-signed certificate, keeps it,
//! and the phone pins it at pairing ([`crate::pairing`]) -- the way an `ssh`
//! `known_hosts` line pins a host key.
//!
//! * **One identity, kept.** Created on first use, `0600`, beside the token file
//!   ([`default_path`]), and read on every start after: a new certificate is a new
//!   pairing, so it must outlive restarts. Its key is the token file's location
//!   (beside the config), never a session id, a pid or an address.
//! * **A leaf.** `CA:FALSE` (explicit), so nothing the key signs is trusted for
//!   anything; the client also pins the leaf itself by SHA-256.
//! * **Which binds speak it.** [`needs_tls`]: every bind that is not loopback.
//!   Loopback stays plain HTTP, and that is the one plaintext path kept: the page
//!   at `/` and `curl` on the same machine, which no other host can observe and
//!   which a browser could not use over a self-signed certificate anyway. A
//!   plaintext request to a TLS port gets no HTTP answer at all.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use rcgen::PublicKeyData as _;
use rustls::pki_types::pem::PemObject as _;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::ServerConfig;
use sha2::{Digest as _, Sha256};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, Semaphore};
use tokio_rustls::server::TlsStream;
use tokio_rustls::TlsAcceptor;

/// The certificate and its key, as `--serve` keeps and serves them.
pub struct Identity {
    cert: CertificateDer<'static>,
    key: PrivatePkcs8KeyDer<'static>,
    spki_sha256: [u8; 32],
}

impl Identity {
    /// The certificate's DER: what the pairing string carries.
    #[must_use]
    pub fn cert_der(&self) -> &[u8] {
        self.cert.as_ref()
    }

    /// SHA-256 of the certificate's DER, 64 lowercase hex digits: the form the
    /// pairing string carries and `openssl x509 -fingerprint -sha256` prints
    /// (without its colons).
    #[must_use]
    pub fn fingerprint(&self) -> String {
        fingerprint(self.cert_der())
    }

    /// `curl --pinnedpubkey`'s form: base64 of the SHA-256 of the public key.
    #[must_use]
    pub fn curl_pin(&self) -> String {
        base64::engine::general_purpose::STANDARD.encode(self.spki_sha256)
    }

    /// The PEM file [`load_or_create`] writes: the certificate, then the key.
    #[must_use]
    pub fn to_pem(&self) -> String {
        let b64 = base64::engine::general_purpose::STANDARD;
        let block = |label: &str, der: &[u8]| {
            let body = b64.encode(der);
            let lines: Vec<&str> = body
                .as_bytes()
                .chunks(PEM_LINE)
                .map(|c| std::str::from_utf8(c).unwrap_or_default())
                .collect();
            format!(
                "-----BEGIN {label}-----\n{}\n-----END {label}-----\n",
                lines.join("\n")
            )
        };
        block("CERTIFICATE", self.cert_der()) + &block("PRIVATE KEY", self.key.secret_pkcs8_der())
    }

    /// The rustls server configuration that presents this certificate.
    ///
    /// # Errors
    ///
    /// When rustls refuses the pair -- among other things, a key that does not
    /// belong to the certificate.
    pub fn server_config(&self) -> Result<Arc<ServerConfig>, String> {
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let mut config = ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .map_err(|e| format!("TLS configuration: {e}"))?
            .with_no_client_auth()
            .with_single_cert(
                vec![self.cert.clone()],
                PrivateKeyDer::Pkcs8(self.key.clone_key()),
            )
            .map_err(|e| format!("the TLS certificate and key do not form a pair: {e}"))?;
        config.alpn_protocols = vec![b"http/1.1".to_vec()];
        Ok(Arc::new(config))
    }
}

/// Characters per base64 line in a PEM block (RFC 7468).
const PEM_LINE: usize = 64;

/// A SHA-256 in hex: two digits per byte.
const FINGERPRINT_HEX: usize = 64;

/// SHA-256 of `der`, 64 lowercase hex digits.
#[must_use]
pub fn fingerprint(der: &[u8]) -> String {
    use std::fmt::Write as _;
    Sha256::digest(der)
        .iter()
        .fold(String::with_capacity(FINGERPRINT_HEX), |mut out, b| {
            let _ = write!(out, "{b:02x}");
            out
        })
}

/// True for every bind but loopback: those are the binds another host can reach.
#[must_use]
pub const fn needs_tls(bind: SocketAddr) -> bool {
    !bind.ip().is_loopback()
}

/// Where the identity lives: beside the token, so one config is one server with
/// one token and one certificate.
#[must_use]
pub fn default_path(token_path: &Path) -> PathBuf {
    token_path.with_file_name("serve-tls.pem")
}

/// A fresh self-signed identity: ECDSA P-256 (the shortest certificate, so the
/// smallest QR code), a leaf with `CA:FALSE`, for server authentication only.
///
/// # Errors
///
/// When key generation or signing fails.
pub fn generate() -> Result<Identity, String> {
    let fail = |e: rcgen::Error| format!("could not make the TLS certificate: {e}");
    let key = rcgen::KeyPair::generate().map_err(fail)?;
    let mut params = rcgen::CertificateParams::new(vec!["multitop".to_string()]).map_err(fail)?;
    params.distinguished_name = rcgen::DistinguishedName::new();
    params
        .distinguished_name
        .push(rcgen::DnType::CommonName, "multitop --serve");
    params.is_ca = rcgen::IsCa::ExplicitNoCa;
    params.key_usages = vec![rcgen::KeyUsagePurpose::DigitalSignature];
    params.extended_key_usages = vec![rcgen::ExtendedKeyUsagePurpose::ServerAuth];
    let cert = params.self_signed(&key).map_err(fail)?;
    Ok(Identity {
        cert: cert.der().clone(),
        key: PrivatePkcs8KeyDer::from(key.serialize_der()),
        spki_sha256: Sha256::digest(key.subject_public_key_info()).into(),
    })
}

/// Parse the PEM [`Identity::to_pem`] writes: exactly one certificate and one
/// PKCS#8 key that belongs to it.
///
/// # Errors
///
/// A sentence naming `path`, and the fix (delete it to make a new one -- which
/// is a new pairing).
pub fn parse(pem: &[u8], path: &Path) -> Result<Identity, String> {
    let shown = path.display();
    let bad = |why: String| {
        format!(
            "{shown}: {why}; delete it and --serve makes a new one (the phone then pairs again)"
        )
    };
    let certs = CertificateDer::pem_slice_iter(pem)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| bad(format!("not a PEM certificate ({e})")))?;
    let [cert] = <[CertificateDer<'static>; 1]>::try_from(certs)
        .map_err(|c| bad(format!("holds {} certificates, not one", c.len())))?;
    let key = PrivatePkcs8KeyDer::from_pem_slice(pem)
        .map_err(|e| bad(format!("no PKCS#8 private key ({e})")))?;
    let pair = rcgen::KeyPair::try_from(&PrivateKeyDer::Pkcs8(key.clone_key()))
        .map_err(|e| bad(format!("unusable private key ({e})")))?;
    let identity = Identity {
        cert,
        key,
        spki_sha256: Sha256::digest(pair.subject_public_key_info()).into(),
    };
    identity.server_config().map_err(bad)?;
    Ok(identity)
}

/// The identity at `path`, and whether this call created it. Owner-only, like the
/// token: an existing file anyone else can open is refused.
///
/// # Errors
///
/// A sentence naming `path`, for a file others can open, one that does not parse,
/// or any I/O failure. There is no fallback to plain HTTP on a LAN bind.
pub fn load_or_create(path: &Path) -> Result<(Identity, bool), String> {
    let make = || generate().map(|i| i.to_pem().into_bytes());
    match crate::owner_file::read_or_create(path, make)? {
        crate::owner_file::Outcome::Existing(pem) => parse(&pem, path).map(|i| (i, false)),
        crate::owner_file::Outcome::Created(pem) => parse(&pem, path).map(|i| (i, true)),
    }
}

/// TLS handshakes in progress at once. Past it a new connection is closed rather
/// than queued: a peer that opens sockets and never handshakes must not be able to
/// hold the server's memory or its other clients hostage.
const HANDSHAKES_IN_FLIGHT: usize = 64;

/// How long one handshake may take before its connection is dropped.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

/// Pause after an accept error that is not the peer's doing (out of descriptors).
const ACCEPT_BACKOFF: Duration = Duration::from_millis(250);

/// An `axum::serve` listener that hands over only connections whose TLS handshake
/// finished. Each handshake runs in its own task, so one slow or silent peer
/// delays nobody else's accept.
pub struct TlsListener {
    ready: mpsc::Receiver<(TlsStream<TcpStream>, SocketAddr)>,
    local: SocketAddr,
    accept_loop: tokio::task::JoinHandle<()>,
}

impl Drop for TlsListener {
    fn drop(&mut self) {
        self.accept_loop.abort();
    }
}

/// Wrap `tcp` so every connection it accepts is TLS under `config`.
///
/// # Errors
///
/// When the listener has no local address.
pub fn listener(tcp: TcpListener, config: Arc<ServerConfig>) -> std::io::Result<TlsListener> {
    let local = tcp.local_addr()?;
    let (tx, ready) = mpsc::channel(HANDSHAKES_IN_FLIGHT);
    let acceptor = TlsAcceptor::from(config);
    let slots = Arc::new(Semaphore::new(HANDSHAKES_IN_FLIGHT));
    let accept_loop = tokio::spawn(async move {
        loop {
            let (stream, peer) = match tcp.accept().await {
                Ok(c) => c,
                Err(e) => {
                    if !matches!(
                        e.kind(),
                        std::io::ErrorKind::ConnectionAborted | std::io::ErrorKind::ConnectionReset
                    ) {
                        tokio::time::sleep(ACCEPT_BACKOFF).await;
                    }
                    continue;
                }
            };
            let Ok(slot) = Arc::clone(&slots).try_acquire_owned() else {
                continue; // saturated: `stream` drops here, closing it
            };
            let (acceptor, tx) = (acceptor.clone(), tx.clone());
            tokio::spawn(async move {
                if let Ok(Ok(tls)) =
                    tokio::time::timeout(HANDSHAKE_TIMEOUT, acceptor.accept(stream)).await
                {
                    drop(slot);
                    let _ = tx.send((tls, peer)).await;
                }
            });
        }
    });
    Ok(TlsListener {
        ready,
        local,
        accept_loop,
    })
}

impl axum::serve::Listener for TlsListener {
    type Io = TlsStream<TcpStream>;
    type Addr = SocketAddr;

    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        match self.ready.recv().await {
            Some(conn) => conn,
            // The accept loop only ends when this listener drops it.
            None => std::future::pending().await,
        }
    }

    fn local_addr(&self) -> std::io::Result<Self::Addr> {
        Ok(self.local)
    }
}

#[cfg(test)]
#[path = "serve_tls_tests.rs"]
mod tests;
