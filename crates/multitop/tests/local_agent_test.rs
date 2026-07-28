use multitop::ssh::{spawn_local_agent, Mode};
use multitop_agent::proto::{decode_packet, Payload};
use multitop_agent::SortBy;
use tokio::io::AsyncReadExt;
use tokio::io::BufReader;

#[tokio::test]
async fn local_agent_streams_binary_packets() {
    let mut child = spawn_local_agent(Mode::Monitor, SortBy::Cpu).expect("spawn local agent");
    let stdout = child.stdout.take().expect("stdout piped");
    let mut reader = BufReader::new(stdout);

    let mut header = [0u8; 8];
    reader.read_exact(&mut header).await.expect("read header");
    assert_eq!(&header[..4], multitop_agent::proto::MAGIC);

    let payload_len = u16::from_le_bytes([header[6], header[7]]) as usize;
    assert!(payload_len > 0);

    let mut payload_bytes = vec![0u8; payload_len];
    reader.read_exact(&mut payload_bytes).await.expect("read payload");

    let mut full = Vec::new();
    full.extend_from_slice(&header);
    full.extend_from_slice(&payload_bytes);

    let payload = decode_packet(&full).expect("decode packet");
    if let Payload::Monitor(snap) = payload {
        assert!(!snap.host.is_empty());
    } else {
        panic!("expected Monitor payload");
    }

    let _ = child.kill().await;
}
