//! Docker transport: endpoint resolution and the minimal HTTP/1.1 client.
//!
//! Re-exported through `docker`, which owns the collection logic on top of
//! this; every public path stays `multitop_agent::docker::...`.

use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::os::unix::net::UnixStream;
use std::time::Duration;

const IO_TIMEOUT: Duration = Duration::from_secs(5);

// ---------------------------------------------------------------- transport

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DockerEndpoint {
    Unix(String),
    Tcp(String),
}

/// Where the daemon lives when `DOCKER_HOST` says nothing.
pub const DEFAULT_SOCKET: &str = "/var/run/docker.sock";

impl DockerEndpoint {
    pub fn from_env() -> Self {
        Self::from_docker_host(std::env::var("DOCKER_HOST").ok().as_deref())
    }

    /// Read a `DOCKER_HOST` value. Anything that is neither `tcp://` nor
    /// `unix://` is taken as a bare socket path, which is what the CLI does.
    pub fn from_docker_host(host: Option<&str>) -> Self {
        match host {
            Some(h) if h.starts_with("tcp://") => DockerEndpoint::Tcp(h.to_string()),
            Some(h) if h.starts_with("unix://") => DockerEndpoint::Unix(
                h.strip_prefix("unix://")
                    .filter(|p| !p.is_empty())
                    .unwrap_or(DEFAULT_SOCKET)
                    .to_string(),
            ),
            Some(h) if !h.trim().is_empty() => DockerEndpoint::Unix(h.to_string()),
            _ => DockerEndpoint::Unix(DEFAULT_SOCKET.to_string()),
        }
    }
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// Decode `Transfer-Encoding: chunked` bodies.
pub fn decode_chunked(body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(body.len());
    let mut pos = 0;
    while pos < body.len() {
        let Some(eol) = find_subslice(&body[pos..], b"\r\n") else {
            break;
        };
        let header = &body[pos..pos + eol];
        let size_txt = std::str::from_utf8(header)
            .unwrap_or("")
            .split(';')
            .next()
            .unwrap_or("");
        let Ok(size) = usize::from_str_radix(size_txt.trim(), 16) else {
            break;
        };
        pos += eol + 2;
        if size == 0 {
            break;
        }
        // Saturating, because `size` is a hex number out of the response and
        // `usize::MAX` parses fine. Adding first and clamping afterwards let the
        // overflow wrap to a value *below* `pos`, so the clamp returned an `end`
        // smaller than the start and the slice below panicked -- the clamp that
        // exists to tolerate a truncated body was itself defeated by the wrap.
        let end = pos.saturating_add(size).min(body.len());
        out.extend_from_slice(&body[pos..end]);
        pos = end + 2; // trailing CRLF
    }
    out
}

/// Minimal HTTP/1.1 GET over a unix domain socket or TCP socket.
///
/// `Connection: close` lets us read to EOF instead of tracking content
/// lengths, and the daemon answers every one of these in a single response.
pub fn http_get_on(endpoint: &DockerEndpoint, path: &str) -> io::Result<Vec<u8>> {
    let req = format!(
        "GET {path} HTTP/1.1\r\nHost: localhost\r\nAccept: application/json\r\nConnection: close\r\n\r\n"
    );
    let mut raw = Vec::with_capacity(crate::consts::HTTP_RESPONSE_CAPACITY);

    match endpoint {
        DockerEndpoint::Unix(sock_path) => {
            let mut stream = UnixStream::connect(sock_path.as_str())?;
            stream.set_read_timeout(Some(IO_TIMEOUT))?;
            stream.set_write_timeout(Some(IO_TIMEOUT))?;
            stream.write_all(req.as_bytes())?;
            stream.flush()?;
            stream.read_to_end(&mut raw)?;
        }
        DockerEndpoint::Tcp(addr) => {
            let clean_addr = addr.trim_start_matches("tcp://");
            let mut stream = TcpStream::connect(clean_addr)?;
            stream.set_read_timeout(Some(IO_TIMEOUT))?;
            stream.set_write_timeout(Some(IO_TIMEOUT))?;
            stream.write_all(req.as_bytes())?;
            stream.flush()?;
            stream.read_to_end(&mut raw)?;
        }
    }

    let split = find_subslice(&raw, b"\r\n\r\n")
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "no header terminator"))?;
    let head = String::from_utf8_lossy(&raw[..split]).to_ascii_lowercase();
    let body = &raw[split + 4..];

    let ok = head
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|c| c.parse::<u16>().ok())
        .is_some_and(|c| (200..300).contains(&c));
    if !ok {
        return Err(io::Error::other(format!(
            "docker api returned {}",
            head.lines().next().unwrap_or("?")
        )));
    }

    if head.contains("transfer-encoding: chunked") {
        Ok(decode_chunked(body))
    } else {
        Ok(body.to_vec())
    }
}

#[cfg(test)]
mod chunked_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::decode_chunked;

    #[test]
    fn a_chunk_size_that_overflows_usize_does_not_panic() {
        // `usize::from_str_radix` accepts this happily. Adding it to `pos` before
        // clamping wrapped below `pos`, so the slice ran backwards and the agent
        // aborted -- and the release profile aborts on panic, so the process
        // died rather than the frame being skipped.
        let body = b"ffffffffffffffff\r\npayload\r\n0\r\n\r\n";
        let out = decode_chunked(body);
        assert!(
            out.len() <= body.len(),
            "a bogus chunk size must not yield more than the body holds"
        );
    }

    #[test]
    fn a_truncated_chunk_yields_what_arrived() {
        // The declared size exceeds what is present, which is what a read
        // timeout mid-response looks like.
        let body = b"ff\r\nshort";
        assert_eq!(decode_chunked(body), b"short");
    }

    #[test]
    fn well_formed_chunks_still_decode() {
        assert_eq!(decode_chunked(b"7\r\npayload\r\n0\r\n\r\n"), b"payload");
        // Two chunks, and a size with a chunk extension after a semicolon.
        assert_eq!(
            decode_chunked(b"3\r\nfoo\r\n3;x=y\r\nbar\r\n0\r\n\r\n"),
            b"foobar"
        );
    }

    #[test]
    fn a_non_hex_size_stops_decoding() {
        assert_eq!(decode_chunked(b"zz\r\nnope\r\n"), b"");
    }
}
