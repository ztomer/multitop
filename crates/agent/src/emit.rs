//! Frame emitters: one snapshot -> one wire frame or one screen of text.
//!
//! Split out of `lib.rs` (2026-09-07) because the line-count ratchet refused a
//! commit that pushed that file from 512 to 527 — it was already grandfathered
//! above the 500-line cap, and the growth was `# Errors` documentation that
//! pedantic clippy correctly asked for. Raising a shrink-only ceiling twice in
//! a row is how such a gate dies, so the file was split instead. `lib.rs` is
//! now back under the cap and its baseline entry is gone rather than raised.
//!
//! These are re-exported from the crate root, so no caller changes.

use crate::{docker, fetch, proto, render, Args};

pub(crate) fn emit_hello<W: std::io::Write>(out: &mut W) -> std::io::Result<()> {
    let hello = proto::Hello::new(crate::consts::AGENT_VERSION.to_string());
    out.write_all(&proto::encode_packet(&proto::Payload::Hello(hello)))?;
    Ok(())
}

/// One fetch frame. On a terminal this is the text-only fallback — the full
/// rendering with distro logos lives in the monitor crate's `fetch_render`.
///
/// # Errors
///
/// Propagates any write error from `w` — a closed pipe is the ordinary case
/// (`monitor | head`), and callers decide whether that is fatal.
pub fn emit_fetch<W: std::io::Write>(
    snap: &fetch::FetchSnapshot,
    cols: usize,
    is_tty: bool,
    pal: &crate::palette_view::PaletteView,
    out: &mut W,
) -> std::io::Result<()> {
    if !is_tty {
        emit_hello(out)?;
        out.write_all(&proto::encode_packet(&proto::Payload::Fetch(snap.clone())))?;
        return out.flush();
    }
    let details = [
        ("OS", &snap.os),
        ("Kernel", &snap.kernel),
        ("Uptime", &snap.uptime),
        ("Host", &snap.host_model),
        ("CPU", &snap.cpu_model),
        ("Memory", &snap.memory_str),
        ("Disk", &snap.disk_str),
    ];
    writeln!(
        out,
        "{}",
        crate::fmt::center_header(&snap.user_host, cols, pal)
    )?;
    for (label, val) in &details {
        writeln!(
            out,
            "  {}{:<7}{}: {}{}{}",
            pal.bold, label, pal.reset, pal.white, val, pal.reset
        )?;
    }
    out.flush()
}

/// One docker frame.
///
/// # Errors
///
/// Propagates any write error from the sink — a closed pipe is the ordinary
/// case (`monitor | head`), and callers decide whether that is fatal.
pub fn emit_docker<W: std::io::Write>(
    host: &str,
    rows: Vec<docker::Row>,
    args: &Args,
    is_tty: bool,
    pal: &crate::palette_view::PaletteView,
    out: &mut W,
) -> std::io::Result<()> {
    if is_tty {
        let frame = docker::render(host, args.cols, args.lines, &rows, pal, args.sort);
        writeln!(out, "{}", frame.join("\n"))?;
    } else {
        emit_hello(out)?;
        let payload = proto::Payload::Docker {
            host: host.to_string(),
            rows,
        };
        out.write_all(&proto::encode_packet(&payload))?;
    }
    out.flush()
}

/// The buffers a monitor frame is written through, kept between frames.
///
/// The repainted screen on a terminal, the packet on a pipe. Each grows only
/// when a frame is bigger than every one before it, so a frame costs no
/// allocation (roadmap M1: a fresh packet buffer per frame was the churn the
/// static agent's allocator kept).
#[derive(Debug, Default)]
pub struct FrameBuf {
    pub text: String,
    pub packet: Vec<u8>,
}

/// One monitor frame, through the caller's kept buffers.
///
/// # Errors
///
/// Propagates any write error from the sink — a closed pipe is the ordinary
/// case (`monitor | head`), and callers decide whether that is fatal.
pub fn emit_monitor<W: std::io::Write>(
    snap: &render::Snapshot,
    args: &Args,
    is_tty: bool,
    pal: &crate::palette_view::PaletteView,
    bufs: &mut FrameBuf,
    out: &mut W,
) -> std::io::Result<()> {
    if !is_tty {
        proto::encode_monitor_packet_into(snap, &mut bufs.packet);
        out.write_all(&bufs.packet)?;
        return out.flush();
    }
    let buf = &mut bufs.text;
    buf.clear();
    buf.push_str("\x1b[H\x1b[J");
    render::render_to_buf(
        snap,
        args.cols,
        args.lines,
        render::bar_len_for(args.cols),
        pal,
        buf,
    );
    out.write_all(buf.as_bytes())?;
    out.flush()
}
