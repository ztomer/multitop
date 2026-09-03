use super::*;

// -------------------------------------------------------------- the handshake

#[tokio::test]
async fn the_handshake_recognises_framing_split_across_reads() {
    // A pipe may hand back fewer bytes than asked for. Comparing what arrived
    // instead of the whole header made the agent's own framing look like text.
    for chunk in [1usize, 2, 3, 4] {
        let magic = multitop_agent::proto::MAGIC;
        let mut reader = ChunkedReader::new(magic.to_vec(), chunk);
        assert_eq!(
            read_handshake(&mut reader).await,
            Handshake::Framed,
            "chunk={chunk}"
        );
    }
}

#[tokio::test]
async fn the_handshake_reads_a_banner_as_text() {
    let mut reader = ChunkedReader::new(b"Welcome to Ubuntu 22.04 LTS\n".to_vec(), 3);
    assert_eq!(
        read_handshake(&mut reader).await,
        Handshake::Text("Welcome to Ubuntu 22.04 LTS".to_string())
    );
}

#[tokio::test]
async fn a_session_that_says_nothing_is_a_close() {
    let mut reader = ChunkedReader::new(Vec::new(), 4);
    assert_eq!(read_handshake(&mut reader).await, Handshake::Closed);
    // Fewer than four bytes is the same: there is no header there.
    let mut short = ChunkedReader::new(b"hi".to_vec(), 1);
    assert_eq!(read_handshake(&mut short).await, Handshake::Closed);
}

/// A reader that hands back at most `chunk` bytes per call, like a pipe.
struct ChunkedReader {
    data: Vec<u8>,
    pos: usize,
    chunk: usize,
}

impl ChunkedReader {
    const fn new(data: Vec<u8>, chunk: usize) -> Self {
        Self {
            data,
            pos: 0,
            chunk,
        }
    }
}

impl tokio::io::AsyncRead for ChunkedReader {
    fn poll_read(
        mut self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let take = self
            .chunk
            .min(buf.remaining())
            .min(self.data.len() - self.pos);
        let (pos, chunk) = (self.pos, take);
        buf.put_slice(&self.data[pos..pos + chunk]);
        self.pos += take;
        std::task::Poll::Ready(Ok(()))
    }
}

impl tokio::io::AsyncBufRead for ChunkedReader {
    fn poll_fill_buf(
        self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<std::io::Result<&[u8]>> {
        let this = self.get_mut();
        let end = (this.pos + this.chunk).min(this.data.len());
        std::task::Poll::Ready(Ok(&this.data[this.pos..end]))
    }
    fn consume(mut self: std::pin::Pin<&mut Self>, amt: usize) {
        self.pos += amt;
    }
}

// --------------------------------------------------------------- the decision

#[test]
fn framing_at_the_handshake_starts_the_reader_with_the_header_it_already_read() {
    assert_eq!(
        bootstrap(Handshake::Framed, "web-01", 0, None),
        Bootstrap::Ready {
            pending_header: Some(*multitop_agent::proto::MAGIC),
            preamble: None,
        }
    );
}

#[test]
fn text_at_the_handshake_is_carried_to_whoever_reports_the_failure() {
    assert_eq!(
        bootstrap(Handshake::Text("banner".into()), "web-01", 0, None),
        Bootstrap::Ready {
            pending_header: None,
            preamble: Some("banner".into()),
        }
    );
}

#[test]
fn a_closed_session_reports_what_stderr_said_when_it_said_anything() {
    assert_eq!(
        bootstrap(
            Handshake::Closed,
            "web-01",
            0,
            Some("Permission denied".into())
        ),
        Bootstrap::Failed("Permission denied".into())
    );
    // With nothing on stderr, all that can honestly be said is that it closed.
    assert_eq!(
        bootstrap(Handshake::Closed, "web-01", 0, None),
        Bootstrap::Failed("Connection to web-01 closed".into())
    );
}

#[test]
fn a_remote_with_no_agent_asks_for_the_build_that_matches_it() {
    assert_eq!(
        bootstrap(Handshake::NeedAgent("x86_64".into()), "h", 0, None),
        Bootstrap::Install(Arch::X86_64)
    );
    assert_eq!(
        bootstrap(Handshake::NeedAgent("aarch64".into()), "h", 0, None),
        Bootstrap::Install(Arch::Aarch64)
    );
}

#[test]
fn an_architecture_multitop_does_not_ship_is_named_in_the_failure() {
    let Bootstrap::Failed(msg) =
        bootstrap(Handshake::NeedAgent("mips64".into()), "web-01", 0, None)
    else {
        panic!("an unshippable architecture must fail");
    };
    assert!(msg.contains("mips64"), "{msg}");
    assert!(msg.contains("web-01"), "{msg}");
    assert!(msg.contains("x86_64 and aarch64"), "{msg}");
}

#[test]
fn an_agent_that_is_still_missing_after_an_install_is_not_installed_again() {
    // Second pass: asking for the agent again means the install did not take,
    // and retrying forever would peg the host.
    assert_eq!(
        bootstrap(Handshake::NeedAgent("x86_64".into()), "web-01", 1, None),
        Bootstrap::Failed("Agent did not start on web-01 after install".into())
    );
}

// ---------------------------------------------------------------- spawn errors

#[test]
fn a_missing_binary_names_the_program_that_is_actually_missing() {
    let local = Server {
        host: "localhost".into(),
        port: 0,
        user: String::new(),
        upgrade_cmd: None,
        custom_command: None,
    };
    let remote = Server {
        host: "web-01".into(),
        port: 22,
        user: "root".into(),
        upgrade_cmd: None,
        custom_command: None,
    };
    let not_found = std::io::Error::new(std::io::ErrorKind::NotFound, "no such file");

    // A local panel never runs `ssh`, so telling the operator to go find one
    // sends them after a program that was working the whole time.
    assert!(spawn_failure(&local, &not_found).contains("multitop-agent binary was not found"));
    assert_eq!(spawn_failure(&remote, &not_found), "ssh command not found");
}

#[test]
fn any_other_spawn_failure_still_names_the_program() {
    let local = Server {
        host: "localhost".into(),
        port: 0,
        user: String::new(),
        upgrade_cmd: None,
        custom_command: None,
    };
    let remote = Server {
        host: "web-01".into(),
        port: 22,
        user: "root".into(),
        upgrade_cmd: None,
        custom_command: None,
    };
    let denied = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied");

    assert!(spawn_failure(&local, &denied).starts_with("could not start multitop-agent"));
    assert!(spawn_failure(&remote, &denied).starts_with("ssh: "));
}

#[tokio::test]
async fn the_reason_on_stderr_survives_stdout_closing_first() {
    // The regression this exists for. `select!` raced the EOF against the
    // stderr line and returned on whichever landed first, so the diagnosis was
    // kept or thrown away depending on how loaded the machine was -- it passed
    // under `cargo test` and failed under the slower instrumented run.
    //
    // Written so stdout closes *before* stderr is written, which is the losing
    // order, and repeated so a single lucky scheduling cannot pass it.
    // `exec 1>&-` closes stdout before anything is written to stderr, so the
    // reader sees EOF first every time. That is the losing order, made certain
    // rather than waited for: as a race it passed under `cargo test` and failed
    // under the slower instrumented run.
    let mut stream =
        stream_from_script("exec 1>&-; sleep 0.05; printf 'Permission denied (publickey)\\n' >&2");
    let mut errbuf = Vec::new();

    assert!(next_packet(&mut stream, &mut errbuf)
        .await
        .unwrap()
        .is_none());
    assert!(
        errbuf.iter().any(|l| l.contains("Permission denied")),
        "the close was reported with no reason: {errbuf:?}"
    );
}

#[tokio::test]
async fn a_close_with_a_silent_stderr_is_still_a_close() {
    // Draining must not turn "nothing to say" into a wait.
    let mut stream = stream_from("", "");
    let mut errbuf = Vec::new();
    assert!(next_packet(&mut stream, &mut errbuf)
        .await
        .unwrap()
        .is_none());
    assert!(
        errbuf.is_empty(),
        "a silent close invented a reason: {errbuf:?}"
    );
}
