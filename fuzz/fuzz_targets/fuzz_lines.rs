#![no_main]

// `proc_read::for_each_line_in`, the one line splitter for the pseudofiles and
// the agent's stdin, against a reference: the input split on newlines, a line
// as long as the buffer or longer passed over, as is a line that is not UTF-8.
// The first two bytes pick the buffer size and how much each read returns, so
// lines straddle reads and buffer ends the way a pipe and a pseudofile make
// them.

use libfuzzer_sys::fuzz_target;
use multitop_agent::proc_read::for_each_line_in;
use multitop_agent::stdin_watch::{parse_size_line, read_commands, size_line, PanelSize};

/// A reader that hands out at most `chunk` bytes a read.
struct Chunked<'a> {
    data: &'a [u8],
    chunk: usize,
}

impl std::io::Read for Chunked<'_> {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        let n = self.data.len().min(out.len()).min(self.chunk);
        out[..n].copy_from_slice(&self.data[..n]);
        self.data = &self.data[n..];
        Ok(n)
    }
}

fuzz_target!(|data: &[u8]| {
    let [buf_len, chunk, input @ ..] = data else {
        return;
    };
    let buf_len = usize::from(*buf_len % 64) + 1;
    let chunk = usize::from(*chunk % 16) + 1;

    let mut expected: Vec<&str> = Vec::new();
    let mut parts: Vec<&[u8]> = input.split(|&b| b == b'\n').collect();
    if parts.last().is_some_and(|last| last.is_empty()) {
        parts.pop();
    }
    for part in parts {
        if part.len() < buf_len {
            if let Ok(s) = std::str::from_utf8(part) {
                expected.push(s);
            }
        }
    }

    let mut got: Vec<String> = Vec::new();
    let mut buf = vec![0u8; buf_len];
    let reader = Chunked { data: input, chunk };
    let any = for_each_line_in(reader, &mut buf, |l| got.push(l.to_owned()));
    assert_eq!(any, !input.is_empty());
    assert_eq!(got, expected);

    // The stdin commands: the panel ends at the last size a held line states,
    // and every size the parser accepts is one the client could have written.
    let panel = PanelSize::default();
    read_commands(Chunked { data: input, chunk }, &panel);
    let last = input
        .split(|&b| b == b'\n')
        .filter(|l| l.len() < multitop_agent::consts::STDIN_WATCH_BUF)
        .filter_map(parse_size_line)
        .last();
    assert_eq!(
        panel.get(),
        last.map(|(c, l)| (usize::from(c), usize::from(l)))
    );
    if let Some((cols, lines)) = parse_size_line(input) {
        assert!(cols > 0 && lines > 0);
        let written = size_line(cols, lines);
        assert_eq!(
            parse_size_line(written.trim_end().as_bytes()),
            Some((cols, lines))
        );
    }
});
