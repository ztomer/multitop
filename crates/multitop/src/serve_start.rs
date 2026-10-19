//! Everything `--serve` settles before it binds: the token, the TLS identity when
//! the bind needs one ([`crate::serve_tls::needs_tls`]), and what it prints.
//!
//! The pairing string carries the token, so it follows the token's rule: printed
//! (with its QR code) only to a terminal. A log gets the certificate's fingerprint,
//! which is not a secret and is what a human compares against the phone.

use std::fmt::Write as _;
use std::net::{IpAddr, SocketAddr};
use std::path::Path;
use std::sync::Arc;

/// A `--serve` ready to bind.
pub struct Ready {
    pub token: String,
    /// `Some` exactly when [`crate::serve_tls::needs_tls`] holds for the bind.
    pub tls: Option<Arc<rustls::ServerConfig>>,
    pub banner: String,
}

/// The token at `token_path`, the TLS identity beside it when `bind` needs one,
/// and the banner.
///
/// `route` answers which local address an unspecified bind is reached at
/// ([`crate::pairing::route_source`] outside tests).
///
/// # Errors
///
/// Any refusal from the token or the identity: there is no fallback to a weaker
/// token or to plain HTTP on a LAN bind.
pub fn prepare(
    bind: SocketAddr,
    token_path: &Path,
    to_tty: bool,
    route: impl FnOnce(bool) -> Option<IpAddr>,
) -> Result<Ready, String> {
    let (token, created) = crate::serve_token::load_or_create(token_path)?;
    if !crate::serve_tls::needs_tls(bind) {
        let banner =
            crate::serve_token::startup_banner(bind, &token, token_path, created, to_tty, None);
        return Ok(Ready {
            token,
            tls: None,
            banner,
        });
    }
    let tls_path = crate::serve_tls::default_path(token_path);
    let (identity, tls_created) = crate::serve_tls::load_or_create(&tls_path)?;
    let mut banner = crate::serve_token::startup_banner(
        bind,
        &token,
        token_path,
        created,
        to_tty,
        Some(&identity),
    );
    let fresh = if tls_created {
        " (new: pair the phone again)"
    } else {
        ""
    };
    let _ = write!(
        banner,
        "\n  certificate: {}{fresh}\n  sha256={}",
        tls_path.display(),
        identity.fingerprint()
    );
    banner += &pairing_lines(bind, &token, identity.cert_der(), to_tty, route);
    Ok(Ready {
        token,
        tls: Some(identity.server_config()?),
        banner,
    })
}

fn pairing_lines(
    bind: SocketAddr,
    token: &str,
    cert_der: &[u8],
    to_tty: bool,
    route: impl FnOnce(bool) -> Option<IpAddr>,
) -> String {
    if !to_tty {
        return "\n  pairing string: printed only to a terminal (it carries the token)".into();
    }
    let Some(addr) = crate::pairing::advertised(bind, route) else {
        return format!(
            "\n  pairing string: unavailable -- {bind} names no address a phone can dial; \
             bind a LAN address instead, e.g. --serve 192.168.1.20:{}",
            bind.port()
        );
    };
    match crate::pairing::payload(addr, token, cert_der) {
        Ok(text) => {
            let qr = crate::pairing::qr(&text).unwrap_or_else(|e| format!("  ({e})"));
            format!("\n  pair the app: {text}\n{qr}")
        }
        Err(why) => format!("\n  pairing string: unavailable -- {why}"),
    }
}

#[cfg(test)]
#[path = "serve_start_tests.rs"]
mod tests;
