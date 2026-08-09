//! The Docker transport, driven against a fake daemon.
//!
//! `http_get_on` is a hand-rolled HTTP/1.1 client; the parts worth pinning are
//! the ones a real daemon exercises and a unit test of the parsers cannot —
//! chunked vs plain bodies, non-2xx replies, and a socket that is not there.
//! The fake speaks just enough HTTP to answer the two paths the agent asks for.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;

use multitop_agent::docker::{
    collect_from, http_get_on, parse_cli_stats, rows_from_cli, rows_from_stats, Container,
    DockerEndpoint, Stats, DEFAULT_SOCKET,
};

/// A unix socket in the temp dir that is removed when the test ends.
struct TempSocket(PathBuf);

impl TempSocket {
    fn new(tag: &str) -> Self {
        static N: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "multitop-fake-docker-{tag}-{}-{}.sock",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_file(&path);
        TempSocket(path)
    }
    fn endpoint(&self) -> DockerEndpoint {
        DockerEndpoint::Unix(self.0.to_string_lossy().into_owned())
    }
}

impl Drop for TempSocket {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Serve `n` connections, answering each with `reply(path)`.
fn serve_unix(
    listener: UnixListener,
    n: usize,
    reply: impl Fn(&str) -> Vec<u8> + Send + 'static,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        for _ in 0..n {
            let Ok((mut sock, _)) = listener.accept() else {
                return;
            };
            let mut req = [0u8; 2048];
            let read = sock.read(&mut req).unwrap_or(0);
            let head = String::from_utf8_lossy(&req[..read]).to_string();
            let path = head.split_whitespace().nth(1).unwrap_or("/").to_string();
            let _ = sock.write_all(&reply(&path));
            let _ = sock.flush();
        }
    })
}

fn plain_response(body: &str) -> Vec<u8> {
    format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n{body}")
        .into_bytes()
}

fn chunked_response(body: &str) -> Vec<u8> {
    let mut out = String::from(
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n",
    );
    out.push_str(&format!("{:x}\r\n{body}\r\n0\r\n\r\n", body.len()));
    out.into_bytes()
}

const CONTAINERS: &str = r#"[
  {"Id":"aaa111","Names":["/web"],"Status":"Up 3 days","Image":"nginx:latest"},
  {"Id":"bbb222","Names":["/db"],"Status":"Up 1 hour","Image":"postgres:16"}
]"#;

fn stats_json(cpu_total: u64, system_total: u64, used: u64, limit: u64) -> String {
    format!(
        r#"{{"cpu_stats":{{"cpu_usage":{{"total_usage":{cpu_total}}},"system_cpu_usage":{system_total},"online_cpus":4}},
            "memory_stats":{{"usage":{used},"limit":{limit},"stats":{{"inactive_file":0}}}}}}"#
    )
}

// ------------------------------------------------------------------ transport

#[test]
fn a_plain_body_comes_back_whole() {
    let sock = TempSocket::new("plain");
    let listener = UnixListener::bind(&sock.0).unwrap();
    let handle = serve_unix(listener, 1, |_| plain_response(CONTAINERS));

    let body = http_get_on(&sock.endpoint(), "/v1.41/containers/json").unwrap();
    assert_eq!(String::from_utf8(body).unwrap(), CONTAINERS);
    handle.join().unwrap();
}

#[test]
fn a_chunked_body_is_decoded_before_it_is_returned() {
    let sock = TempSocket::new("chunked");
    let listener = UnixListener::bind(&sock.0).unwrap();
    let handle = serve_unix(listener, 1, |_| chunked_response(CONTAINERS));

    let body = http_get_on(&sock.endpoint(), "/v1.41/containers/json").unwrap();
    assert_eq!(String::from_utf8(body).unwrap(), CONTAINERS);
    handle.join().unwrap();
}

#[test]
fn a_non_success_status_is_an_error_rather_than_an_empty_list() {
    // Reading the body of a 404 as JSON would silently produce "no
    // containers", which is indistinguishable from a healthy idle host.
    let sock = TempSocket::new("404");
    let listener = UnixListener::bind(&sock.0).unwrap();
    let handle = serve_unix(listener, 1, |_| {
        b"HTTP/1.1 404 Not Found\r\nConnection: close\r\n\r\n{\"message\":\"no such endpoint\"}"
            .to_vec()
    });

    let err = http_get_on(&sock.endpoint(), "/v1.41/containers/json").unwrap_err();
    assert!(
        err.to_string().contains("404"),
        "error should name the status: {err}"
    );
    handle.join().unwrap();
}

#[test]
fn a_reply_with_no_header_terminator_is_an_error() {
    let sock = TempSocket::new("noterm");
    let listener = UnixListener::bind(&sock.0).unwrap();
    let handle = serve_unix(listener, 1, |_| b"HTTP/1.1 200 OK".to_vec());

    assert!(http_get_on(&sock.endpoint(), "/v1.41/containers/json").is_err());
    handle.join().unwrap();
}

#[test]
fn a_socket_that_is_not_there_is_an_error_not_a_hang() {
    let endpoint = DockerEndpoint::Unix("/tmp/multitop-no-such-docker.sock".into());
    assert!(http_get_on(&endpoint, "/v1.41/containers/json").is_err());
}

#[test]
fn a_tcp_endpoint_is_dialled_over_tcp() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = thread::spawn(move || {
        let (mut sock, _) = listener.accept().unwrap();
        let mut req = [0u8; 1024];
        let _ = sock.read(&mut req);
        let _ = sock.write_all(&plain_response(CONTAINERS));
    });

    let endpoint = DockerEndpoint::Tcp(format!("tcp://{addr}"));
    let body = http_get_on(&endpoint, "/v1.41/containers/json").unwrap();
    assert_eq!(String::from_utf8(body).unwrap(), CONTAINERS);
    handle.join().unwrap();
}

#[test]
fn an_unreachable_tcp_endpoint_is_an_error() {
    // Port 1 on loopback refuses rather than accepting.
    let endpoint = DockerEndpoint::Tcp("tcp://127.0.0.1:1".into());
    assert!(http_get_on(&endpoint, "/v1.41/containers/json").is_err());
}

// ------------------------------------------------------------------ endpoint

#[test]
fn docker_host_selects_the_transport() {
    assert_eq!(
        DockerEndpoint::from_docker_host(Some("tcp://10.0.0.1:2375")),
        DockerEndpoint::Tcp("tcp://10.0.0.1:2375".into())
    );
    assert_eq!(
        DockerEndpoint::from_docker_host(Some("unix:///run/user/1000/docker.sock")),
        DockerEndpoint::Unix("/run/user/1000/docker.sock".into())
    );
    // A bare path is taken as a socket, which is what the CLI accepts too.
    assert_eq!(
        DockerEndpoint::from_docker_host(Some("/run/docker.sock")),
        DockerEndpoint::Unix("/run/docker.sock".into())
    );
}

#[test]
fn an_absent_or_empty_docker_host_falls_back_to_the_standard_socket() {
    let default = DockerEndpoint::Unix(DEFAULT_SOCKET.into());
    assert_eq!(DockerEndpoint::from_docker_host(None), default);
    assert_eq!(DockerEndpoint::from_docker_host(Some("")), default);
    assert_eq!(DockerEndpoint::from_docker_host(Some("   ")), default);
    // `unix://` with nothing after it names no socket, so the default stands
    // rather than the agent dialling the empty path.
    assert_eq!(DockerEndpoint::from_docker_host(Some("unix://")), default);
}

#[test]
fn the_environment_is_read_for_the_endpoint() {
    // Whatever this machine's DOCKER_HOST says, `from_env` must agree with
    // reading that same value directly.
    let expected = DockerEndpoint::from_docker_host(std::env::var("DOCKER_HOST").ok().as_deref());
    assert_eq!(DockerEndpoint::from_env(), expected);
}

// ---------------------------------------------------------------- collection

#[test]
fn a_full_collection_pass_reports_a_row_per_container() {
    // One list request plus two stats requests per container, twice over
    // (the CPU percentage needs two samples).
    let sock = TempSocket::new("collect");
    let listener = UnixListener::bind(&sock.0).unwrap();
    let served = AtomicUsize::new(0);
    let handle = serve_unix(listener, 5, move |path| {
        if path.contains("/stats") {
            let n = served.fetch_add(1, Ordering::Relaxed) as u64;
            // Later samples report more CPU, so a percentage comes out.
            plain_response(&stats_json(
                1_000_000 * (n + 1),
                10_000_000 * (n + 1),
                128 << 20,
                512 << 20,
            ))
        } else {
            plain_response(CONTAINERS)
        }
    });

    let mut rows = collect_from(&sock.endpoint());
    handle.join().unwrap();

    rows.sort_by(|a, b| a.name.cmp(&b.name));
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].name, "db");
    assert_eq!(rows[1].name, "web");
    assert_eq!(rows[1].status, "Up 3 days");
    for r in &rows {
        assert!(
            r.cpu.ends_with('%'),
            "cpu column must be a percentage: {}",
            r.cpu
        );
        assert_eq!(r.mem_bytes, 128 << 20);
    }
}

#[test]
fn a_daemon_that_lists_nothing_yields_no_rows_and_asks_for_no_stats() {
    let sock = TempSocket::new("empty");
    let listener = UnixListener::bind(&sock.0).unwrap();
    // Exactly one connection is served: a stats request would block here and
    // the join below would never return.
    let handle = serve_unix(listener, 1, |_| plain_response("[]"));

    assert!(collect_from(&sock.endpoint()).is_empty());
    handle.join().unwrap();
}

#[test]
fn an_unreachable_daemon_falls_through_to_the_cli() {
    // With no daemon and (on this host) no `docker` binary either, the answer
    // is an empty table rather than a panic or a hang.
    let endpoint = DockerEndpoint::Unix("/tmp/multitop-no-such-docker-collect.sock".into());
    let _ = collect_from(&endpoint);
}

// ------------------------------------------------------------ row assembly

#[test]
fn a_container_the_stats_pass_missed_still_gets_a_row() {
    let containers = vec![
        Container {
            id: "aaa".into(),
            name: "web".into(),
            status: "Up".into(),
            image: "n".into(),
        },
        Container {
            id: "bbb".into(),
            name: "db".into(),
            status: "Up".into(),
            image: "p".into(),
        },
    ];
    let mut stats = HashMap::new();
    stats.insert(
        "aaa".to_string(),
        Stats {
            cpu_pct: 12.34,
            mem_used: 100,
            mem_limit: 200,
        },
    );

    let rows = rows_from_stats(containers, &stats);
    assert_eq!(rows.len(), 2, "a container with no stats must not vanish");
    assert_eq!(rows[0].cpu, "12.3%");
    assert_eq!(rows[0].mem, "100B/200B");
    // No stats: zeroes, and an unset limit prints as "-" rather than "0B/0B".
    assert_eq!(rows[1].cpu, "0.0%");
    assert_eq!(rows[1].mem, "-");
    assert_eq!(rows[1].mem_bytes, 0);
}

#[test]
fn an_unconstrained_container_shows_no_memory_limit() {
    let containers = vec![Container {
        id: "aaa".into(),
        name: "web".into(),
        status: "Up".into(),
        image: "n".into(),
    }];
    let mut stats = HashMap::new();
    stats.insert(
        "aaa".to_string(),
        Stats {
            cpu_pct: 0.0,
            mem_used: 4096,
            mem_limit: 0,
        },
    );
    assert_eq!(rows_from_stats(containers, &stats)[0].mem, "-");
}

#[test]
fn the_cli_fallback_joins_its_two_tables_on_the_container_name() {
    let ps = "web\tUp 3 days\tnginx\taaa111\ndb\tUp 1 hour\tpostgres\tbbb222\n";
    let stats = parse_cli_stats("web\t12.5%\t128MiB / 512MiB\n");

    let rows = rows_from_cli(ps, &stats);
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].name, "web");
    assert_eq!(rows[0].cpu, "12.5%");
    assert!((rows[0].cpu_pct - 12.5).abs() < 0.001);
    assert_eq!(rows[0].mem, "128MiB / 512MiB");
    // A container the stats table did not mention reads as idle, not missing.
    assert_eq!(rows[1].name, "db");
    assert_eq!(rows[1].cpu, "0.0%");
    assert_eq!(rows[1].cpu_pct, 0.0);
    assert_eq!(rows[1].mem, "0B / 0B");
}

#[test]
fn a_cli_percentage_that_will_not_parse_reads_as_zero() {
    let stats = parse_cli_stats("web\t--\t128MiB / 512MiB\n");
    let rows = rows_from_cli("web\tUp\tnginx\taaa\n", &stats);
    assert_eq!(rows[0].cpu_pct, 0.0);
    // The text is still shown verbatim rather than being replaced by 0.0%.
    assert_eq!(rows[0].cpu, "--");
}

#[test]
fn an_empty_cli_listing_yields_no_rows() {
    assert!(rows_from_cli("", &HashMap::new()).is_empty());
}
