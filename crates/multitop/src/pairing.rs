//! The pairing string `--serve` prints for the Android companion.
//!
//! **The client owns this format** (docs/roadmap.md, A2.1):
//!
//! ```text
//! multitop://pair?v=1&addr=<host>:<port>&token=<64 hex>&sha256=<64 hex>&cert=<base64url DER>
//! ```
//!
//! and parses it strictly (`android/.../pairing/PairingPayload.kt`): every field
//! exactly once, nothing unknown, one spelling of each. [`check`] holds this side to
//! the same rules, and [`payload`] runs it on everything it builds, so the server
//! never prints a string the phone would refuse -- it says why instead. The golden
//! pair under `android/app/src/test/resources/` (`rust_server.pem`,
//! `rust_server_pairing.txt`) is written by this module and read by the Kotlin
//! tests, so neither half can drift alone.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use base64::Engine as _;

/// What every pairing string starts with.
pub const PREFIX: &str = "multitop://pair?";
/// The one version this server writes and the client reads.
pub const VERSION: &str = "1";
const FIELDS: [&str; 5] = ["v", "addr", "token", "sha256", "cert"];
const HEX_LEN: usize = 64;
/// The client's bound on `cert`: a P-256 certificate is ~530 characters.
const MAX_CERT_CHARS: usize = 4096;

/// A pairing string's fields, as the client reads them.
#[derive(Debug, PartialEq, Eq)]
pub struct Fields {
    pub host: String,
    pub port: u16,
    pub token: String,
    pub sha256: String,
    pub cert_der: Vec<u8>,
}

fn is_lower_hex(s: &str) -> bool {
    s.len() == HEX_LEN && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// `<host>:<port>` by the client's rule: a DNS name or IPv4 address, or an IPv6
/// address in brackets, and a port from 1 to 65535.
fn address(addr: &str) -> Option<(String, u16)> {
    let (host, port) = addr.rsplit_once(':')?;
    let port = port
        .bytes()
        .all(|b| b.is_ascii_digit())
        .then(|| port.parse::<u16>().ok())
        .flatten()
        .filter(|p| *p != 0)?;
    let ipv6 = |b: u8| b.is_ascii_hexdigit() || b == b':' || b == b'.';
    let name = |b: u8| b.is_ascii_alphanumeric() || b == b'.' || b == b'-';
    let (bare, allowed): (&str, &dyn Fn(u8) -> bool) = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .map_or((host, &name), |inner| (inner, &ipv6));
    let bare = (!bare.is_empty() && bare.bytes().all(allowed)).then_some(bare)?;
    Some((bare.to_string(), port))
}

/// Read `text` by the rules the client enforces.
///
/// # Errors
///
/// The client's reason for refusing it.
pub fn check(text: &str) -> Result<Fields, String> {
    let rest = text
        .trim()
        .strip_prefix(PREFIX)
        .ok_or_else(|| format!("not a multitop pairing string (expected {PREFIX}...)"))?;
    let mut fields = HashMap::new();
    for part in rest.split('&') {
        let (key, value) = part.split_once('=').unwrap_or(("", ""));
        if !FIELDS.contains(&key) {
            return Err(format!(
                "unknown or malformed field: {}",
                part.split('=').next().unwrap_or("")
            ));
        }
        if fields.insert(key, value).is_some() {
            return Err(format!("field given twice: {key}"));
        }
    }
    if let Some(missing) = FIELDS.iter().find(|f| !fields.contains_key(*f)) {
        return Err(format!("missing field: {missing}"));
    }
    let get = |k: &str| fields.get(k).copied().unwrap_or_default();
    if get("v") != VERSION {
        return Err(format!(
            "unsupported pairing version {} (the client reads {VERSION})",
            get("v")
        ));
    }
    if !is_lower_hex(get("token")) {
        return Err("token must be 64 lowercase hex digits".into());
    }
    if !is_lower_hex(get("sha256")) {
        return Err("sha256 must be 64 lowercase hex digits, without colons".into());
    }
    let (host, port) =
        address(get("addr")).ok_or("addr must be <host>:<port> with a port from 1 to 65535")?;
    let cert = get("cert");
    let cert_der = (cert.len() <= MAX_CERT_CHARS
        && cert
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'))
    .then(|| {
        base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(cert)
            .ok()
    })
    .flatten()
    .filter(|der| !der.is_empty())
    .ok_or("cert is not unpadded base64url")?;
    if crate::serve_tls::fingerprint(&cert_der) != get("sha256") {
        return Err("sha256 does not match cert: the pairing string is damaged or altered".into());
    }
    Ok(Fields {
        host,
        port,
        token: get("token").to_string(),
        sha256: get("sha256").to_string(),
        cert_der,
    })
}

/// The pairing string for a phone reaching the server at `addr`.
///
/// # Errors
///
/// Why the client would refuse it -- in practice a hand-written token that is not
/// 64 lowercase hex digits, which the string cannot carry.
pub fn payload(addr: SocketAddr, token: &str, cert_der: &[u8]) -> Result<String, String> {
    let cert = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(cert_der);
    let sha256 = crate::serve_tls::fingerprint(cert_der);
    // `SocketAddr`'s Display brackets an IPv6 host; rebuilt from the bare IP so a
    // scope id (`%en0`), which the client's address rule refuses, never appears.
    let addr = SocketAddr::new(addr.ip(), addr.port());
    let text = format!("{PREFIX}v={VERSION}&addr={addr}&token={token}&sha256={sha256}&cert={cert}");
    check(&text).map(|_| text)
}

/// The address a phone should dial for a server bound at `bind`.
///
/// A specific address is itself. An unspecified one (`0.0.0.0`, `::`) listens
/// everywhere and names nowhere, so it is the address `route` reports this host
/// would send from -- `None` when there is none, and then no pairing string.
pub fn advertised(
    bind: SocketAddr,
    route: impl FnOnce(bool) -> Option<IpAddr>,
) -> Option<SocketAddr> {
    if bind.ip().is_unspecified() {
        let ip = route(bind.is_ipv6()).filter(|ip| !ip.is_unspecified() && !ip.is_loopback())?;
        return Some(SocketAddr::new(ip, bind.port()));
    }
    Some(bind)
}

/// The local address this host would route to a documentation-range peer from
/// (RFC 5737 / RFC 3849). A UDP `connect` only consults the routing table: no
/// packet is sent.
#[must_use]
pub fn route_source(v6: bool) -> Option<IpAddr> {
    let (local, peer): (SocketAddr, SocketAddr) = if v6 {
        (
            (Ipv6Addr::UNSPECIFIED, 0).into(),
            (Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1), 9).into(),
        )
    } else {
        (
            (Ipv4Addr::UNSPECIFIED, 0).into(),
            (Ipv4Addr::new(192, 0, 2, 1), 9).into(),
        )
    };
    let socket = std::net::UdpSocket::bind(local).ok()?;
    socket.connect(peer).ok()?;
    socket.local_addr().ok().map(|a| a.ip())
}

/// `text` as a QR code drawn in the terminal, two modules per character cell.
///
/// # Errors
///
/// When `text` is too long for any QR code.
pub fn qr(text: &str) -> Result<String, String> {
    let code = qrcode::QrCode::with_error_correction_level(text.as_bytes(), qrcode::EcLevel::L)
        .map_err(|e| format!("cannot draw the pairing QR code: {e}"))?;
    Ok(code
        .render::<qrcode::render::unicode::Dense1x2>()
        .quiet_zone(true)
        .build())
}

#[cfg(test)]
#[path = "pairing_tests.rs"]
mod tests;
