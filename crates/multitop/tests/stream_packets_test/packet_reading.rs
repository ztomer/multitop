use super::*;

#[tokio::test]
async fn a_framed_packet_is_read_and_decoded() {
    let mut stream = stream_from_bytes(&packet("web-01"), b"");
    let mut errbuf = Vec::new();

    let payload = next_packet(&mut stream, &mut errbuf)
        .await
        .expect("read must succeed")
        .expect("a payload must be there");
    let Payload::Monitor(snap) = payload else {
        panic!("wrong payload kind");
    };
    assert_eq!(snap.host, "web-01");
    assert!(
        errbuf.is_empty(),
        "a clean read must report nothing: {errbuf:?}"
    );
}

#[tokio::test]
async fn packets_are_read_back_to_back_until_the_stream_ends() {
    let mut bytes = packet("a");
    bytes.extend_from_slice(&packet("b"));
    let mut stream = stream_from_bytes(&bytes, b"");
    let mut errbuf = Vec::new();

    for expected in ["a", "b"] {
        let Some(Payload::Monitor(snap)) = next_packet(&mut stream, &mut errbuf).await.unwrap()
        else {
            panic!("expected a monitor packet");
        };
        assert_eq!(snap.host, expected);
    }
    // End of stream is `None`, not an error — the caller turns that into a
    // closed connection.
    assert!(next_packet(&mut stream, &mut errbuf)
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn a_header_the_handshake_already_consumed_is_not_read_twice() {
    // `connect` reads four bytes to recognise the framing and hands them back
    // through `pending_header`; reading eight more here would take the first
    // half of the payload as the rest of the header.
    let mut stream = stream_from_bytes(&packet("web-01")[4..], b"");
    stream.pending_header = Some(*multitop_agent::proto::MAGIC);
    let mut errbuf = Vec::new();

    let Some(Payload::Monitor(snap)) = next_packet(&mut stream, &mut errbuf).await.unwrap() else {
        panic!("expected a monitor packet");
    };
    assert_eq!(snap.host, "web-01");
}

#[tokio::test]
async fn a_stream_that_ends_inside_a_pending_header_is_a_close_not_an_error() {
    let mut stream = stream_from_bytes(&[0x00, 0x01], b"");
    stream.pending_header = Some(*multitop_agent::proto::MAGIC);
    let mut errbuf = Vec::new();
    assert!(next_packet(&mut stream, &mut errbuf)
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn a_stream_that_produces_nothing_at_all_is_a_close() {
    let mut stream = stream_from("", "");
    let mut errbuf = Vec::new();
    assert!(next_packet(&mut stream, &mut errbuf)
        .await
        .unwrap()
        .is_none());
    assert_eq!(errbuf, [] as [std::string::String; 0]);
}

#[tokio::test]
async fn text_where_the_framing_should_be_is_reported_as_that() {
    // The message has to say the agent never started, not that the connection
    // closed: the host is up and talking.
    let mut stream = stream_from("Welcome to Ubuntu 22.04\n", "");
    stream.preamble = Some("Welcome to Ubuntu 22.04".to_string());
    let mut errbuf = Vec::new();

    let err = next_packet(&mut stream, &mut errbuf).await.unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    assert!(
        errbuf.iter().any(|l| l.contains("Welcome to Ubuntu")),
        "{errbuf:?}"
    );
    assert!(
        errbuf.iter().any(|l| l.contains("login banner")),
        "{errbuf:?}"
    );
}

#[tokio::test]
async fn framing_lost_mid_stream_is_reported_as_that() {
    // A good packet, then eight bytes that are not a header. No preamble, so
    // this is the "was working, then desynchronised" wording.
    let mut bytes = packet("web-01");
    bytes.extend_from_slice(b"NOTMAGIC");
    let mut stream = stream_from_bytes(&bytes, b"");
    let mut errbuf = Vec::new();

    assert!(next_packet(&mut stream, &mut errbuf)
        .await
        .unwrap()
        .is_some());
    let err = next_packet(&mut stream, &mut errbuf).await.unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    assert!(
        errbuf.iter().any(|l| l.contains("framing was lost")),
        "{errbuf:?}"
    );
    assert!(
        errbuf.iter().any(|l| l.contains("host is reachable")),
        "the operator must not be sent to look at the network: {errbuf:?}"
    );
}

#[tokio::test]
async fn a_payload_cut_short_is_an_error_the_panel_can_report() {
    // Header promises a payload; the stream stops halfway through it.
    let full = packet("web-01");
    let mut stream = stream_from_bytes(&full[..full.len() - 4], b"");
    let mut errbuf = Vec::new();

    let err = next_packet(&mut stream, &mut errbuf).await.unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::UnexpectedEof);
    assert!(
        errbuf
            .iter()
            .any(|l| l.contains("reading from the agent failed")),
        "a truncated payload must leave a reason behind: {errbuf:?}"
    );
}

#[tokio::test]
async fn stderr_that_arrives_while_waiting_for_a_header_is_kept() {
    // The remote writes a warning and then nothing else. The reader must
    // collect the warning rather than blocking on stdout with it unread.
    let mut stream = stream_from("", "Warning: Permanently added a host\n");
    let mut errbuf = Vec::new();

    assert!(next_packet(&mut stream, &mut errbuf)
        .await
        .unwrap()
        .is_none());
    assert!(
        errbuf.iter().any(|l| l.contains("Permanently added")),
        "stderr arriving before EOF must be kept: {errbuf:?}"
    );
}

// ------------------------------------------------------------ the error buffer

#[test]
fn the_reason_buffer_keeps_the_most_recent_lines_and_no_more() {
    let mut errbuf = Vec::new();
    for i in 0..MAX_STDERR_LINES * 2 {
        note(&mut errbuf, format!("line {i}"));
    }
    assert_eq!(errbuf.len(), MAX_STDERR_LINES, "the bound was not applied");
    // The oldest are dropped, so the last thing that happened survives.
    assert_eq!(
        errbuf.last().unwrap(),
        &format!("line {}", MAX_STDERR_LINES * 2 - 1)
    );
    assert!(!errbuf.iter().any(|l| l == "line 0"));
}

#[test]
fn a_framing_failure_is_described_without_being_prefixed() {
    // `framing_lost` already says what happened and what to do; wrapping it
    // would bury that behind a second sentence.
    let e = framing_lost(b"NOTMAGIC", None);
    assert_eq!(describe_failure(&e), e.to_string());

    let with_text = framing_lost(b"Welcome!", Some("Welcome to Ubuntu".into()));
    assert!(with_text.to_string().contains("Welcome to Ubuntu"));
    assert!(with_text.to_string().contains("never started"));
}

#[test]
fn any_other_read_failure_says_the_host_may_still_be_up() {
    let e = std::io::Error::new(std::io::ErrorKind::BrokenPipe, "pipe went away");
    let described = describe_failure(&e);
    assert!(described.contains("reading from the agent failed"));
    assert!(described.contains("pipe went away"));
    assert!(described.contains("may still be reachable"));
}

#[test]
fn a_packet_this_build_cannot_read_is_not_reported_as_a_closed_connection() {
    let mut errbuf = Vec::new();
    // Intact framing, unknown mode byte: the stream is fine, the dialect is not.
    let mut pkt = packet("web-01");
    pkt[5] = 99;
    assert!(interpret_packet(&pkt, 99, pkt.len() - 8, &mut errbuf).is_none());
    assert!(
        errbuf.iter().any(|l| l.contains("different version")),
        "{errbuf:?}"
    );
    assert!(errbuf.iter().any(|l| l.contains("mode 99")), "{errbuf:?}");

    // A packet that decodes leaves no complaint behind.
    let mut clean = Vec::new();
    let good = packet("web-01");
    assert!(interpret_packet(&good, good[5], good.len() - 8, &mut clean).is_some());
    assert_eq!(clean, [] as [std::string::String; 0]);
}
