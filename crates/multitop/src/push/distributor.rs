//! A self-hosted `UnifiedPush` distributor that wants a credential.
//!
//! A public distributor (ntfy.sh) takes an anonymous POST to the endpoint the
//! phone registered. A private one does not: the fleet's ntfy (servers
//! DECISIONS #17) is `deny-all`, so its `up*` topics accept a publish only with
//! the publisher's token, and it is served under a private CA that Mozilla's
//! roots do not hold.
//!
//! So the owner may install `push-distributor.toml`, `0600`, beside the server's
//! `config.toml`:
//!
//! ```toml
//! origin = "https://ntfy.media.home"   # the distributor's scheme://host[:port]
//! token  = "tk_..."                    # sent as a bearer, to this origin only
//! ca     = """-----BEGIN CERTIFICATE-----
//! ...
//! -----END CERTIFICATE-----"""          # optional: extra roots, this origin only
//! ```
//!
//! Both the token and the extra roots are scoped to that ONE origin: an
//! endpoint anywhere else (another distributor, FCM) gets neither, so a private
//! CA cannot vouch for Google and the token never leaves for a host it was not
//! issued by. Without the file nothing changes.

use std::path::{Path, PathBuf};

use rustls::pki_types::pem::PemObject as _;
use rustls::pki_types::CertificateDer;
use serde::Deserialize;

use super::http::{target, Request};

/// The file, beside `config.toml`.
pub const DISTRIBUTOR_FILE: &str = "push-distributor.toml";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    origin: String,
    token: String,
    #[serde(default)]
    ca: Option<String>,
}

/// The distributor's origin, its token and its extra roots. Never `Debug`-prints
/// the token.
#[derive(Clone)]
pub struct Distributor {
    host: String,
    port: u16,
    tls: bool,
    token: String,
    roots: Vec<CertificateDer<'static>>,
}

impl std::fmt::Debug for Distributor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Distributor")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("roots", &self.roots.len())
            .finish_non_exhaustive()
    }
}

/// Where the file lives for a config at `config_path`.
#[must_use]
pub fn default_path(config_path: &Path) -> PathBuf {
    config_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(DISTRIBUTOR_FILE)
}

impl Distributor {
    /// Parse the file's text. Every error names the field, never its value.
    ///
    /// # Errors
    ///
    /// Not TOML of this shape, an origin that is not a URL push would accept
    /// (or carries a path), an empty or non-printable token, or a `ca` with no
    /// certificate in it.
    pub fn parse(text: &str) -> Result<Self, String> {
        let file: File =
            toml::from_str(text).map_err(|_| "is not origin/token/ca TOML".to_string())?;
        let (uri, host, port, tls) = target(&file.origin).map_err(|e| format!("origin: {e}"))?;
        if uri.path_and_query().is_some_and(|p| p.as_str() != "/") {
            return Err("origin: must be scheme://host[:port], without a path".into());
        }
        let printable = file.token.bytes().all(|b| b.is_ascii_graphic());
        if file.token.is_empty() || !printable {
            return Err("token: empty or not a printable ASCII token".into());
        }
        let roots = match &file.ca {
            None => Vec::new(),
            Some(pem) => {
                let certs: Vec<_> = CertificateDer::pem_slice_iter(pem.as_bytes())
                    .collect::<Result<_, _>>()
                    .map_err(|_| "ca: is not PEM certificates".to_string())?;
                if certs.is_empty() {
                    return Err("ca: holds no certificate".into());
                }
                certs
            }
        };
        Ok(Self {
            host: host.to_ascii_lowercase(),
            port,
            tls,
            token: file.token,
            roots,
        })
    }

    /// Whether `url` is on this distributor's origin.
    #[must_use]
    pub fn serves(&self, url: &str) -> bool {
        target(url).is_ok_and(|(_, host, port, tls)| {
            host.eq_ignore_ascii_case(&self.host) && port == self.port && tls == self.tls
        })
    }

    /// The request with the bearer added when it goes to this origin, and
    /// untouched otherwise.
    #[must_use]
    pub fn authorize(&self, mut req: Request) -> Request {
        if self.serves(&req.url) {
            req.headers
                .retain(|(k, _)| !k.eq_ignore_ascii_case("authorization"));
            req.headers
                .push(("authorization".into(), format!("Bearer {}", self.token)));
        }
        req
    }

    /// The extra trust anchors for this origin (empty: Mozilla's alone).
    #[must_use]
    pub fn roots(&self) -> &[CertificateDer<'static>] {
        &self.roots
    }
}

/// The owner's distributor, or `None` when no file is installed.
///
/// # Errors
///
/// A file that is open to others (it holds a credential), unreadable, or
/// malformed - named by path and field, never by value.
pub fn load(path: &Path) -> Result<Option<Distributor>, String> {
    let meta = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("{}: {e}", path.display())),
    };
    crate::owner_file::refuse_if_open(
        path,
        std::os::unix::fs::PermissionsExt::mode(&meta.permissions()),
    )?;
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    Distributor::parse(&text)
        .map(Some)
        .map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
#[path = "distributor_tests.rs"]
mod tests;
