use crate::stream::prod::*;

mod spawn_failure_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::spawn_failure;
    use crate::config::Server;

    fn server(host: &str, port: u16) -> Server {
        Server {
            host: host.to_string(),
            port,
            user: "admin".to_string(),
            upgrade_cmd: None,
            custom_command: None,
        }
    }

    /// A local panel never runs `ssh`. Blaming `ssh` for a missing agent binary
    /// sends the user to check an `ssh` that is installed and working.
    #[test]
    fn a_local_panel_blames_the_agent_binary_not_ssh() {
        let missing = std::io::Error::from(std::io::ErrorKind::NotFound);
        let msg = spawn_failure(&server("localhost", 0), &missing);
        assert!(
            msg.contains("multitop-agent"),
            "the message must name what was actually missing: {msg}"
        );
        assert!(
            !msg.contains("ssh"),
            "and must not send the user after ssh: {msg}"
        );
    }

    /// A remote panel does run `ssh`, so that one keeps its message.
    #[test]
    fn a_remote_panel_still_blames_ssh() {
        let missing = std::io::Error::from(std::io::ErrorKind::NotFound);
        let msg = spawn_failure(&server("db-02", 22), &missing);
        assert!(msg.contains("ssh"), "{msg}");
    }

    /// Failures that are not "missing program" keep their detail, and still
    /// name the right program.
    #[test]
    fn other_failures_keep_their_detail() {
        let denied = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        let local = spawn_failure(&server("localhost", 0), &denied);
        assert!(local.contains("multitop-agent"), "{local}");
        let remote = spawn_failure(&server("db-02", 22), &denied);
        assert!(remote.starts_with("ssh:"), "{remote}");
    }
}

#[cfg(test)]
mod packet_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::interpret_packet;
    use multitop_agent::proto;

    /// A packet that arrived intact and could not be read is not a closed
    /// connection.
    ///
    /// It used to become one: `decode_packet` returns `None` for a mode byte
    /// this build does not know, `next_packet` handed that straight back as
    /// `Ok(None)`, and every caller turns `Ok(None)` into
    /// `Connection to <host> closed`. The operator was sent to look at the
    /// network for a host that was up and talking.
    #[test]
    fn an_unreadable_packet_says_so_rather_than_claiming_the_host_went_away() {
        let mut packet = Vec::from(*proto::MAGIC);
        // version, an unknown mode, and a zero-length body.
        packet.extend_from_slice(&[1, 200, 0, 0]);
        let mut errbuf = Vec::new();

        let payload = interpret_packet(&packet, 200, 0, &mut errbuf);

        assert!(payload.is_none(), "an unknown mode cannot be decoded");
        assert_eq!(errbuf.len(), 1, "and it must leave a reason behind");
        assert!(
            errbuf[0].contains("mode 200") && errbuf[0].contains("host is reachable"),
            "the reason must name the mode and clear the host: {:?}",
            errbuf[0]
        );
    }

    /// The reason goes in the same bounded buffer as the stderr lines, so a
    /// stream that produces nothing but unreadable packets cannot grow it
    /// without limit.
    #[test]
    fn the_reason_respects_the_buffer_bound() {
        let mut packet = Vec::from(*proto::MAGIC);
        packet.extend_from_slice(&[1, 200, 0, 0]);
        let mut errbuf = Vec::new();
        for _ in 0..(super::MAX_STDERR_LINES * 3) {
            interpret_packet(&packet, 200, 0, &mut errbuf);
        }
        assert_eq!(errbuf.len(), super::MAX_STDERR_LINES);
    }

    /// A remote that printed a banner instead of running the agent must be
    /// reported as exactly that.
    ///
    /// The three readers of this stream all pattern-match
    /// `while let Ok(Some(payload))`, so the `Err` this returns ends their loop
    /// and is dropped. What they report is built from `errbuf`: the monitor
    /// takes its last line and otherwise says `Connection to <host> closed`,
    /// and fetch and docker say only what is in it. So the reason has to be in
    /// `errbuf`, not only in the error.
    #[test]
    fn a_banner_where_the_framing_should_be_is_not_reported_as_a_closed_connection() {
        let err = super::framing_lost(b"Welcome ", Some("Welcome to Ubuntu 24.04".to_string()));

        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
        let reason = super::describe_failure(&err);
        assert!(
            reason.contains("Welcome to Ubuntu 24.04"),
            "the reason must quote what the remote actually said: {reason:?}"
        );
        assert!(
            reason.contains("login banner"),
            "and name the cause the operator can act on: {reason:?}"
        );
    }

    /// A desync with no banner behind it still says the host is reachable,
    /// rather than leaving the panel to claim it went away.
    #[test]
    fn a_mid_stream_desync_still_clears_the_host() {
        let err = super::framing_lost(&[0xff; 8], None);
        assert!(
            super::describe_failure(&err).contains("host is reachable"),
            "{err}"
        );
    }

    /// The three siblings the framing fix left behind: the two short header
    /// reads and the payload read all return a plain I/O error, and every one of
    /// them was silent. Fixing only the framing case would have left an I/O
    /// error mid-stream stopping the monitor with `Connection to <host> closed`
    /// and stopping fetch and docker with nothing at all.
    #[test]
    fn a_plain_io_failure_is_described_rather_than_left_bare() {
        for kind in [
            std::io::ErrorKind::BrokenPipe,
            std::io::ErrorKind::ConnectionReset,
            std::io::ErrorKind::TimedOut,
        ] {
            let reason = super::describe_failure(&std::io::Error::from(kind));
            assert!(
                reason.contains("reading from the agent failed"),
                "{kind:?} must arrive as something an operator can read: {reason:?}"
            );
            assert!(
                reason.contains("may still be reachable"),
                "{kind:?} must not read as the host going away: {reason:?}"
            );
        }
    }

    /// Both reason paths and the stderr path share one bound. They used to be
    /// written twice with two different rules -- `>` and `>=` -- so the buffer
    /// held nine lines or eight depending on which kind of line arrived last.
    #[test]
    fn every_path_into_the_buffer_respects_one_bound() {
        let mut errbuf = Vec::new();
        let framing = super::framing_lost(&[0xff; 8], None);
        for i in 0..(super::MAX_STDERR_LINES * 2) {
            super::note(&mut errbuf, format!("stderr {i}"));
            super::note(&mut errbuf, super::describe_failure(&framing));
        }
        assert_eq!(errbuf.len(), super::MAX_STDERR_LINES);
    }

    /// A packet that decodes is handed back untouched, with nothing added to
    /// the buffer that reports failures.
    #[test]
    fn a_readable_packet_is_returned_and_reports_nothing() {
        let payload = proto::Payload::Fetch(multitop_agent::fetch::FetchSnapshot::default());
        let encoded = proto::encode_packet(&payload);
        let len = encoded.len() - 8;
        let mut errbuf = Vec::new();

        let got = interpret_packet(&encoded, encoded[5], len, &mut errbuf);

        assert!(got.is_some(), "a well-formed packet must decode");
        assert!(errbuf.is_empty(), "and must report nothing: {errbuf:?}");
    }
}

#[cfg(test)]
mod handshake_tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::{read_handshake, Handshake};
    use tokio::io::{AsyncRead, ReadBuf};

    /// A reader that hands back one byte at a time, which a pipe is entitled to
    /// do and which the old four-bytes-in-one-`read` handshake could not
    /// survive.
    struct Dribble {
        bytes: Vec<u8>,
        at: usize,
    }

    impl AsyncRead for Dribble {
        fn poll_read(
            mut self: std::pin::Pin<&mut Self>,
            _cx: &mut std::task::Context<'_>,
            buf: &mut ReadBuf<'_>,
        ) -> std::task::Poll<std::io::Result<()>> {
            if self.at < self.bytes.len() && buf.remaining() > 0 {
                let b = self.bytes[self.at];
                self.at += 1;
                buf.put_slice(&[b]);
            }
            std::task::Poll::Ready(Ok(()))
        }
    }

    fn dribbled(bytes: &[u8]) -> tokio::io::BufReader<Dribble> {
        // Capacity 1, so the `BufReader` cannot paper over the split the way a
        // default-sized one would.
        tokio::io::BufReader::with_capacity(
            1,
            Dribble {
                bytes: bytes.to_vec(),
                at: 0,
            },
        )
    }

    /// The regression. A framed stream whose magic arrives a byte at a time is
    /// still a framed stream; read four bytes at a time with `read`, it was
    /// mistaken for text, and every packet after it read from the wrong offset.
    #[tokio::test]
    async fn a_magic_header_split_across_reads_is_still_recognised() {
        let mut framed = Vec::from(*multitop_agent::proto::MAGIC);
        framed.extend_from_slice(&[0, 0, 4, 0]);
        let mut reader = dribbled(&framed);
        assert_eq!(read_handshake(&mut reader).await, Handshake::Framed);
    }

    #[tokio::test]
    async fn a_need_agent_line_names_its_architecture() {
        let mut reader = dribbled(b"===NEEDAGENT=== aarch64\n");
        assert_eq!(
            read_handshake(&mut reader).await,
            Handshake::NeedAgent("aarch64".to_string())
        );
    }

    /// A banner is left for the packet reader to fail on -- and it must be
    /// carried, not dropped. Dropped, the reader fails on the *next* eight
    /// bytes with `invalid magic header` and the panel can only say the
    /// connection closed, about a host that is up and printing a banner.
    #[tokio::test]
    async fn a_banner_is_carried_so_the_failure_can_name_it() {
        let mut reader = dribbled(b"Welcome to Ubuntu 24.04\n");
        assert_eq!(
            read_handshake(&mut reader).await,
            Handshake::Text("Welcome to Ubuntu 24.04".to_string()),
            "the text that made this not-framed has to survive the handshake"
        );
    }

    /// Fewer than four bytes and then nothing is a closed connection, not a
    /// three-byte banner to go hunting through.
    #[tokio::test]
    async fn a_stream_that_closes_early_is_closed() {
        assert_eq!(read_handshake(&mut dribbled(b"")).await, Handshake::Closed);
        assert_eq!(
            read_handshake(&mut dribbled(b"hi")).await,
            Handshake::Closed
        );
    }
}
