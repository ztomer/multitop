use std::sync::OnceLock;

use multitop_agent::color::Palette;
use multitop_agent::fetch::FetchSnapshot;
use multitop_agent::fmt::center_header;

/// A logo from the neofetch database.
#[derive(Debug)]
struct Logo {
    patterns: Vec<String>,
    colors: Vec<u8>,
    lines: Vec<String>,
}

/// Parsed logo database.
struct LogoDb {
    logos: Vec<Logo>,
}

static LOGO_DB: OnceLock<LogoDb> = OnceLock::new();

fn load_db() -> &'static LogoDb {
    LOGO_DB.get_or_init(|| {
        let compressed = include_bytes!("../data/logos.bin.zst");
        let raw = zstd::decode_all(&compressed[..]).expect("zstd decompress logo db");
        parse_db(&raw)
    })
}

fn parse_db(bytes: &[u8]) -> LogoDb {
    assert!(bytes.len() >= 8, "logo db too short");
    assert_eq!(&bytes[..4], b"MTLG", "bad magic");
    let _version = u16::from_le_bytes([bytes[4], bytes[5]]);
    let count = u16::from_le_bytes([bytes[6], bytes[7]]);
    let mut pos = 8;

    let mut logos = Vec::with_capacity(count as usize);

    for _ in 0..count {
        let entry_size = u16::from_le_bytes([bytes[pos], bytes[pos + 1]]) as usize;
        let entry_end = pos + 2 + entry_size;
        pos += 2;

        let num_patterns = bytes[pos] as usize;
        pos += 1;

        let mut patterns = Vec::with_capacity(num_patterns);
        for _ in 0..num_patterns {
            let plen = bytes[pos] as usize;
            pos += 1;
            patterns.push(String::from_utf8_lossy(&bytes[pos..pos + plen]).to_string());
            pos += plen;
        }

        let num_colors = bytes[pos] as usize;
        pos += 1;
        let colors: Vec<u8> = bytes[pos..pos + num_colors].to_vec();
        pos += num_colors;

        let num_lines = u16::from_le_bytes([bytes[pos], bytes[pos + 1]]) as usize;
        pos += 2;

        let mut lines = Vec::with_capacity(num_lines);
        for _ in 0..num_lines {
            let llen = bytes[pos] as usize;
            pos += 1;
            lines.push(strip_color_markers(&bytes[pos..pos + llen]));
            pos += llen;
        }

        logos.push(Logo {
            patterns,
            colors,
            lines,
        });

        pos = entry_end;
    }

    LogoDb { logos }
}

fn color_for(ci: u8, pal: &Palette) -> &'static str {
    match ci {
        1 => pal.red,
        2 => pal.green,
        3 => pal.yellow,
        4 => pal.blue,
        5 => pal.purple,
        6 => pal.cyan,
        7 => pal.white,
        _ => pal.white,
    }
}

fn strip_color_markers(bytes: &[u8]) -> String {
    let s = String::from_utf8_lossy(bytes);
    let mut result = String::with_capacity(s.len());
    let s_bytes = s.as_bytes();
    let mut last = 0;
    let mut i = 0;
    while i < s_bytes.len() {
        if s_bytes[i] == b'$'
            && i + 3 < s_bytes.len()
            && s_bytes[i + 1] == b'{'
            && s_bytes[i + 2] == b'c'
            && s_bytes[i + 3].is_ascii_digit()
        {
            result.push_str(&s[last..i]);
            i += 3;
            while i < s_bytes.len() && s_bytes[i].is_ascii_digit() {
                i += 1;
            }
            if i < s_bytes.len() && s_bytes[i] == b'}' {
                i += 1;
                last = i;
                continue;
            }
        } else {
            i += 1;
        }
    }
    result.push_str(&s[last..]);
    result
}

fn find_logo<'a>(
    db: &'a LogoDb,
    os: &str,
    kernel: &str,
    max_logo_width: usize,
) -> Option<&'a Logo> {
    let o = os.to_ascii_lowercase();
    let k = kernel.to_ascii_lowercase();

    let mut best_fit: Option<&'a Logo> = None;
    let mut best_fit_width: usize = 0;

    for logo in &db.logos {
        for pat in &logo.patterns {
            let pl = pat.as_str();
            if !pl.is_empty() && (o.starts_with(pl) || k.starts_with(pl)) {
                let logo_width = logo
                    .lines
                    .iter()
                    .map(|l| l.chars().count())
                    .max()
                    .unwrap_or(0);
                if logo_width <= max_logo_width && logo_width > best_fit_width {
                    best_fit = Some(logo);
                    best_fit_width = logo_width;
                }
                break;
            }
        }
    }
    best_fit
}

/// Crop the center `target` lines of a logo when we don't have room for all of it.
fn pick_lines(logo: &Logo, target: usize) -> Vec<&str> {
    if logo.lines.len() <= target {
        return logo.lines.iter().map(|s| s.as_str()).collect();
    }
    let start = (logo.lines.len() - target) / 2;
    logo.lines[start..start + target]
        .iter()
        .map(|s| s.as_str())
        .collect()
}

pub fn render_fetch(
    snap: &FetchSnapshot,
    cols: usize,
    max_rows: usize,
    pal: &Palette,
) -> Vec<String> {
    let mut out = Vec::with_capacity(12);
    out.push(center_header(&snap.user_host, cols, pal));

    let db = load_db();

    let details: [(&str, &str); 7] = [
        ("OS", &snap.os),
        ("Kernel", &snap.kernel),
        ("Uptime", &snap.uptime),
        ("Host", &snap.host_model),
        ("CPU", &snap.cpu_model),
        ("Memory", &snap.memory_str),
        ("Disk", &snap.disk_str),
    ];

    let max_val_len = details
        .iter()
        .map(|(_, v)| v.chars().count())
        .max()
        .unwrap_or(0);
    // Overhead = 1 (space before logo) + 1 (space after logo) + 7 (label) + 3 (" : ") + max_val_len
    let detail_overhead = 12 + max_val_len;
    let max_logo_width = cols.saturating_sub(detail_overhead);
    let logo = find_logo(db, &snap.os, &snap.kernel, max_logo_width);

    let colors_row = format!(
        "\x1b[40m  \x1b[41m  \x1b[42m  \x1b[43m  \x1b[44m  \x1b[45m  \x1b[46m  \x1b[47m  {}",
        pal.reset
    );

    let max_body = max_rows.saturating_sub(1);

    if let Some(lg) = logo {
        let accent = color_for(lg.colors.first().copied().unwrap_or(7), pal);
        let logo_lines = pick_lines(lg, max_body);
        let logo_width = logo_lines
            .iter()
            .map(|l| l.chars().count())
            .max()
            .unwrap_or(0);
        let total_rows = logo_lines.len().max(details.len()).min(max_body);

        for i in 0..total_rows {
            let logo_part = logo_lines.get(i).copied().unwrap_or("");
            if i < details.len() {
                let (label, val) = &details[i];
                out.push(format!(
                    " {}{:logo_width$}{}{} {}{:<7}{} : {}{}{}",
                    accent,
                    logo_part,
                    pal.reset,
                    "",
                    pal.primary(),
                    label,
                    pal.reset,
                    pal.text(),
                    val,
                    pal.reset,
                    logo_width = logo_width,
                ));
            } else {
                out.push(format!(
                    " {}{:logo_width$}{}{}",
                    accent,
                    logo_part,
                    pal.reset,
                    "",
                    logo_width = logo_width,
                ));
            }
        }

        if total_rows < max_body {
            out.push(format!("   {}", colors_row));
        }
    } else {
        let mut row_idx = 0;
        for (label, val) in &details {
            if row_idx >= max_body {
                break;
            }
            out.push(format!(
                "   {}{}{:<7}{} : {}{}{}",
                pal.primary(),
                pal.bold,
                label,
                pal.reset,
                pal.text(),
                val,
                pal.reset
            ));
            row_idx += 1;
        }
        if row_idx < max_body {
            out.push(format!("   {}", colors_row));
        }
    }

    out
}
