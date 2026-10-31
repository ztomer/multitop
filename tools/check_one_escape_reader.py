#!/usr/bin/env python3
"""Fail if shipped code READS terminal escapes anywhere but the shared tokenizer.

Three hand-rolled readers of tool output each understood a different subset of
the escape grammar. The upgrade painter knew only `ESC[`; the span renderer
dropped the ESC of `ESC 7` and printed the `7`; `strip_ansi` leaked the `B` of
`ESC(B`. apt draws its progress bar between `ESC 7` and `ESC 8`, so the upgrade
window tore on every package (roadmap N1, 2026-10-10). The fix was one
tokenizer -- `crates/agent/src/escape.rs`, over `vte` -- and this keeps it one:
a fourth reader is how the class comes back, and nothing about writing
`if c == '\\x1b'` says a reader already exists.

The rule: inside `crates/*/src`, outside `escape.rs` and outside test code,
no ESC byte is compared, matched, split on, searched for or stripped. WRITING
escapes (`format!("\\x1b[36m{x}")`) is fine; only reading them is the class.

Test code is exempt: `*_tests.rs` files, and everything from a file's first
`#[cfg(test)]` on. Run with --self-test to check the checker.
"""

from __future__ import annotations

import re
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from _scope import scope_is_empty

ROOT = Path("crates")
OWNER = Path("crates/agent/src/escape.rs")

# An ESC literal, in any of the spellings Rust allows.
_ESC = r"(?:b?'\\(?:x1[bB]|u\{1[bB]\}|e)'|\"\\(?:x1[bB]|u\{1[bB]\})|0x1[bB]\b|27u8\b)"
READ = re.compile(
    rf"(?:==|!=)\s*{_ESC}"  # c == '\x1b'
    rf"|{_ESC}\s*(?:==|!=|=>|\|)"  # '\x1b' => ..., b'\x1b' | ...
    rf"|\.(?:split|split_once|splitn|rsplit|find|rfind|position|strip_prefix|strip_suffix"
    rf"|starts_with|ends_with|contains|matches|trim_start_matches|trim_end_matches"
    rf"|split_inclusive)\(\s*(?:\|[^|]*\|\s*[^)]*)?{_ESC}"
)


def scanned(root: Path) -> list[Path]:
    return sorted(p for p in root.glob("*/src/**/*.rs") if not p.name.endswith("_tests.rs"))


def offenders(root: Path, owner: Path) -> list[tuple[Path, int, str]]:
    hits: list[tuple[Path, int, str]] = []
    for path in scanned(root):
        if path.resolve() == owner.resolve():
            continue
        for lineno, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
            if line.strip().startswith("#[cfg(test)]"):
                break
            code = line.split("//", 1)[0]
            if READ.search(code):
                hits.append((path, lineno, line.strip()))
    return hits


def self_test() -> int:
    cases_red = [
        "if c == '\\x1b' { skip() }",
        "while bytes[i] != 0x1b {",
        "b'\\x1b' => state = Esc,",
        "'\\u{1b}' | '\\x07' => {}",
        "for part in s.split('\\x1b') {",
        'let Some(rest) = s.strip_prefix("\\x1b[") else { return };',
        "let i = text.rfind('\\x1b')?;",
        'if line.starts_with("\\u{1b}[") {',
        "let n = bytes.iter().position(|&b| b == 0x1B);",
    ]
    cases_clean = [
        'out.push_str(&format!("\\x1b[36m{x}\\x1b[0m"));',
        "// if c == '\\x1b' { a comment }",
        'const RESET: &str = "\\x1b[0m";',
    ]
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        src = root / "agent" / "src"
        src.mkdir(parents=True)
        owner = src / "escape.rs"
        owner.write_text("if c == '\\x1b' { reader() }", encoding="utf-8")
        for case in cases_red:
            (src / "x.rs").write_text(case, encoding="utf-8")
            if not offenders(root, owner):
                print(f"self-test FAILED: a hand-rolled read was not detected: {case}")
                return 1
        for case in cases_clean:
            (src / "x.rs").write_text(case, encoding="utf-8")
            if offenders(root, owner):
                print(f"self-test FAILED: not a read, but reported: {case}")
                return 1
        (src / "x.rs").write_text(
            "fn f() {}\n#[cfg(test)]\nmod t { fn g(c: char) -> bool { c == '\\x1b' } }", encoding="utf-8"
        )
        if offenders(root, owner):
            print("self-test FAILED: test code was reported")
            return 1
        (src / "x_tests.rs").write_text("assert!(c == '\\x1b');", encoding="utf-8")
        (src / "x.rs").write_text("fn f() {}", encoding="utf-8")
        if offenders(root, owner):
            print("self-test FAILED: a *_tests.rs file was reported")
            return 1
    print("check_one_escape_reader self-test: ok")
    return 0


def main() -> int:
    if "--self-test" in sys.argv:
        return self_test()
    if not OWNER.is_file():
        print(f"one-escape-reader: {OWNER} not found -- run from the repository root")
        return 1
    files = scanned(ROOT)
    if scope_is_empty("one-escape-reader", len(files), "Rust files under crates/*/src"):
        return 1
    hits = offenders(ROOT, OWNER)
    if not hits:
        print(f"one-escape-reader: clean ({len(files)} files checked)")
        return 0
    print("one-escape-reader: terminal escapes are read outside the shared tokenizer\n")
    for path, lineno, line in hits:
        print(f"  {path}:{lineno}")
        print(f"    {line[:100]}")
    print(
        "\nRead tool output through `multitop_agent::escape` (`scan`, `strip`,\n"
        "`is_complete`): it consumes every sequence whole. A reader of your own\n"
        "will know a subset of the grammar, and the rest leaks into the screen --\n"
        "which is how apt's progress bar tore the upgrade window."
    )
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
