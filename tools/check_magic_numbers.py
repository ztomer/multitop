#!/usr/bin/env python3
"""Flag numeric literals that carry meaning nobody wrote down.

A magic number is not "any number in the source". A palette entry, an SGR code
in a table that maps codes to colours, a `+ 1` for a gutter -- those explain
themselves where they sit, and a check that flagged them would report nine
hundred sites, be switched off within a week, and catch nothing. This project
has said that about its own gates more than once: a gate that cries wolf gets
switched off.

What it flags instead is the shape where the number *is* the behaviour and the
reader has to go and find out why:

    threshold   a comparison against a bare number -- `pct >= 80.0`. Which is
                "high"? The number is the policy, and the policy has no name.
    timeout     `Duration::from_secs(5)`. How long is the wait, and why that
                long?
    array       `[0u8; 256]`. A buffer sized to something the author knew and
                the next reader does not -- and a size that has to agree with
                whatever fills it.
    capacity    `with_capacity(4096)`, `nth(11)`, `truncate(8)`. A field index
                into `/proc/stat` is the sharpest case: `nth(11)` is utime, and
                nothing on that line says so.
    binding     `let budget = 32;` -- the roadmap's own example.
    exit        an exit code, which another program's script branches on.

Deliberately NOT flagged: 0, 1, 2, 3 and 4 (arithmetic and small offsets that
read as themselves), anything inside a string literal (format widths are not
magic), `const`/`static` declarations (naming one is the fix, not the offence),
attributes, comments, and `#[cfg(test)]` regions.

Usage:
    python3 tools/check_magic_numbers.py [--self-test]

Exit status is 1 when something is flagged, so it can gate a commit.

To silence a legitimate case, put a `magic-ok:` note on the line above saying
what the number is and why naming it would not help -- the same escape hatch
`check_test_only_code.py` offers with `reachability:`.
"""

from __future__ import annotations

import re
import sys
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent

# Numbers that read as themselves wherever they appear. Anything larger is a
# quantity somebody chose.
ALLOWED = {"0", "1", "2", "3", "4"}

_N = r"(\d[\d_]*)(?:u8|u16|u32|u64|usize|i8|i16|i32|i64|isize|f32|f64)?"

RULES: list[tuple[str, re.Pattern[str]]] = [
    ("timeout", re.compile(rf"Duration::from_\w+\(\s*{_N}")),
    # `(?<![<>])` and `(?![<>])` so a shift is not read as a comparison:
    # `15 << 16` matched the second `<` and reported 16 as a threshold.
    ("threshold", re.compile(rf"(?<![<>])[<>]=?(?![<>])\s*{_N}(?:\.0)?\b")),
    ("equality", re.compile(rf"[!=]=\s*{_N}(?:\.0)?\b")),
    ("binding", re.compile(rf"\blet\s+(?:mut\s+)?\w+(?:\s*:\s*[\w:<>]+)?\s*=\s*{_N}\s*;")),
    ("capacity", re.compile(rf"\b(?:with_capacity|truncate|nth)\(\s*{_N}\s*\)")),
    ("array", re.compile(rf"\[\s*0u8\s*;\s*{_N}\s*\]")),
    ("exit", re.compile(rf"\bexit\(\s*{_N}\s*\)")),
]

NOTE = re.compile(r"magic-ok:")
DECL = re.compile(r"^\s*(?:pub(?:\(\w+\))?\s+)?(?:const|static)\s")


def strip_test_regions(text: str) -> str:
    """Blank out `#[cfg(test)]` items, preserving line numbering.

    Brace matching rather than a regex, because a test module contains nested
    braces and string literals with braces in them.
    """
    out = list(text)
    for m in re.finditer(r"#\[cfg\(test\)\]", text):
        i = text.find("{", m.end())
        if i == -1 or text.count("\n", m.end(), i) > 3:
            continue
        depth, j = 0, i
        while j < len(text):
            if text[j] == "{":
                depth += 1
            elif text[j] == "}":
                depth -= 1
                if depth == 0:
                    break
            j += 1
        for k in range(m.start(), min(j + 1, len(text))):
            if out[k] != "\n":
                out[k] = " "
    return "".join(out)


def blank_string_contents(line: str) -> str:
    """Replace the inside of every string literal with spaces.

    A format width (`{:<7}`) and a sentinel's own text are not magic numbers,
    and leaving them in was the single largest source of noise in the first
    version of this check.
    """
    out = list(line)
    i, n = 0, len(line)
    while i < n:
        if line[i] == '"':
            j = i + 1
            while j < n:
                if line[j] == "\\":
                    j += 2
                    continue
                if line[j] == '"':
                    break
                out[j] = " "
                j += 1
            i = j + 1
        else:
            i += 1
    return "".join(out)


def rust_files() -> list[Path]:
    """Production Rust, excluding the `*_tests.rs` modules `#[path]` pulls in."""
    seen: set[Path] = set()
    for pattern in ("crates/*/src/**/*.rs", "crates/*/src/*.rs"):
        seen.update(p for p in REPO.glob(pattern) if p.is_file())
    return sorted(p for p in seen if not p.name.endswith("_tests.rs"))


def scan(text: str) -> list[tuple[int, str, str, str]]:
    """(line number, rule, value, source line) for each flagged literal."""
    found = []
    lines = strip_test_regions(text).splitlines()
    for idx, raw in enumerate(lines):
        stripped = raw.strip()
        if not stripped or stripped.startswith(("//", "#[")):
            continue
        if DECL.match(raw):
            continue
        # The escape hatch sits on the line above, where an explanation has
        # room to be an explanation.
        if idx > 0 and NOTE.search(lines[idx - 1]):
            continue
        line = blank_string_contents(raw)
        for name, rx in RULES:
            m = rx.search(line)
            if not m:
                continue
            value = m.group(1).replace("_", "")
            if value in ALLOWED:
                continue
            found.append((idx + 1, name, value, stripped))
            break
    return found


def offenders(root: Path) -> list[tuple[Path, int, str, str, str]]:
    out = []
    for path in rust_files():
        for line_no, rule, value, source in scan(
            path.read_text(encoding="utf-8", errors="replace")
        ):
            out.append((path.relative_to(root), line_no, rule, value, source))
    return out


CLEAN = """\
pub const HIGH_PCT: f64 = 80.0;
pub const BUF: usize = 256;
fn f(pct: f64) -> bool {
    let mut buf = [0u8; BUF];
    let _ = &mut buf;
    // magic-ok: the wire format fixes this at four bytes; naming it would
    // only move the number one line up.
    let header = [0u8; 4];
    let _ = header;
    pct >= HIGH_PCT
}
"""

DIRTY = """\
fn f(pct: f64) -> bool {
    let mut buf = [0u8; 256];
    let _ = &mut buf;
    pct >= 80.0
}
"""

# A shift is not a comparison. `15 << 16` used to be reported as a threshold of
# 16, which is the shape of false positive that gets a gate switched off.
SHIFTS = """\
pub const PAGE: u32 = 15;
fn f() -> u32 {
    PAGE << 16
}
"""


def self_test() -> int:
    """Prove the check still detects before trusting it to report clean."""
    with tempfile.TemporaryDirectory() as tmp:
        clean = Path(tmp) / "clean.rs"
        clean.write_text(CLEAN, encoding="utf-8")
        if scan(CLEAN):
            print("magic-numbers self-test: clean source was flagged", file=sys.stderr)
            for hit in scan(CLEAN):
                print(f"  {hit}", file=sys.stderr)
            return 1
        if scan(SHIFTS):
            print(
                "magic-numbers self-test: a bit shift was read as a comparison",
                file=sys.stderr,
            )
            return 1
        hits = scan(DIRTY)
        rules = {rule for _, rule, _, _ in hits}
        if {"threshold", "array"} - rules:
            print(
                "magic-numbers self-test: dirty source was NOT flagged "
                f"(saw {sorted(rules)})",
                file=sys.stderr,
            )
            return 1
    print("magic-numbers self-test: passed")
    return 0


def main() -> int:
    if "--self-test" in sys.argv:
        return self_test()

    found = offenders(REPO)
    if not found:
        print(f"magic-numbers: clean ({len(rust_files())} files checked)")
        return 0

    print("magic-numbers: literals whose meaning is not written down\n")
    for path, line_no, rule, value, source in found:
        print(f"  {path}:{line_no}: [{rule} {value}] {source}")
    print(
        "\nGive each one a named constant. The name is the documentation:\n"
        "  `if pct >= METER_HIGH_PCT` says what 80.0 was for; `if pct >= 80.0`\n"
        "  makes the next reader guess, and a second site that guesses a\n"
        "  different number is how two thresholds drift apart.\n"
        "\n"
        "If a number genuinely reads as itself, put a `magic-ok:` note on the\n"
        "line above saying what it is and why a name would not help."
    )
    return 1


if __name__ == "__main__":
    sys.exit(main())
