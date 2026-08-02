#!/usr/bin/env python3
"""Fail if tracked text files contain decorative emoji.

House rule: emoji are a failure state. The only permitted pictographs are the
Susan Kare icon set used for TUI output, plus the Mac key glyphs:

    permitted:  arrows and marks  from ICONS below
                Mac key glyphs    from KEYS below

Everything else in the emoji/pictograph Unicode ranges is rejected.

Usage:
    python3 tools/check_no_emoji.py             # check every tracked file
    python3 tools/check_no_emoji.py FILE...     # check specific files
    python3 tools/check_no_emoji.py --staged    # check staged content only
    python3 tools/check_no_emoji.py --self-test # verify the checker detects

Exits 0 when clean, 1 when a violation is found, 2 on a usage error.
"""

from __future__ import annotations

import re
import subprocess
import sys
import unicodedata

# The Kare icon set: the only pictographic characters allowed in this repo.
ICONS = set("→·✓✗⚠↔↑↓←")
# Functional Mac key glyphs, permitted in docs and help text.
KEYS = set("⌘⌥⌨⇧⎋⏎")
ALLOWED = ICONS | KEYS

# Unicode blocks that carry emoji and decorative pictographs.
RANGES = [
    (0x1F000, 0x1FAFF),  # tiles, emoticons, transport, symbols, extended-A
    (0x1F900, 0x1F9FF),  # supplemental symbols and pictographs
    (0x2600, 0x27BF),    # misc symbols + dingbats
    (0x2B00, 0x2BFF),    # misc symbols and arrows
    (0xFE0F, 0xFE0F),    # variation selector-16 (forces emoji presentation)
    (0x1F1E6, 0x1F1FF),  # regional indicators (flags)
]

# Binary and vendored paths that are not ours to police.
SKIP_SUFFIXES = (
    ".png", ".jpg", ".jpeg", ".gif", ".ico", ".pdf", ".zip", ".gz",
    ".woff", ".woff2", ".ttf", ".otf", ".bin", ".lock",
)
SKIP_DIRS = ("target/", "node_modules/", ".git/")


# Escaped forms that render as a character without containing it literally.
# A Rust escape for a padlock codepoint is plain ASCII in the file, so scanning
# characters alone reports a clean tree while the UI draws an emoji. Two were
# hiding in the config panel exactly this way. Rust uses the brace form; the
# four-hex-digit form is here so the same gate works on JS/JSON/Java if any
# lands. (Deliberately described rather than shown -- this file is scanned too.)
ESCAPE_RE = re.compile(r"\\u\{([0-9a-fA-F]{1,6})\}|\\u([0-9a-fA-F]{4})")


def escaped_violations(line: str) -> list[tuple[int, str]]:
    """Codepoints written as escapes that would be forbidden if written out."""
    out = []
    for m in ESCAPE_RE.finditer(line):
        hex_digits = m.group(1) or m.group(2)
        try:
            ch = chr(int(hex_digits, 16))
        except (ValueError, OverflowError):
            continue
        if is_forbidden(ch):
            out.append((m.start() + 1, ch))
    return out


def is_forbidden(ch: str) -> bool:
    if ch in ALLOWED:
        return False
    cp = ord(ch)
    return any(lo <= cp <= hi for lo, hi in RANGES)


def tracked_files() -> list[str]:
    out = subprocess.run(
        ["git", "ls-files"], capture_output=True, text=True, check=True
    ).stdout
    return [p for p in out.splitlines() if p]


def staged_files() -> list[str]:
    out = subprocess.run(
        ["git", "diff", "--cached", "--name-only", "--diff-filter=ACMR"],
        capture_output=True, text=True, check=True,
    ).stdout
    return [p for p in out.splitlines() if p]


def should_skip(path: str) -> bool:
    return path.endswith(SKIP_SUFFIXES) or any(d in path for d in SKIP_DIRS)


def read_staged(path: str) -> str | None:
    """The staged content of `path`, which under partial staging differs from
    what is on disk. Returns None if it is not text or cannot be read."""
    proc = subprocess.run(
        ["git", "show", f":{path}"], capture_output=True, check=False
    )
    if proc.returncode != 0:
        return None
    try:
        return proc.stdout.decode("utf-8")
    except UnicodeDecodeError:
        return None


def check(path: str, staged: bool = False) -> list[str]:
    """Return a list of human-readable violations for one file."""
    if should_skip(path):
        return []
    if staged:
        content = read_staged(path)
        if content is None:
            return []
        lines = content.splitlines(keepends=True)
    else:
        try:
            with open(path, encoding="utf-8") as fh:
                lines = fh.readlines()
        except (OSError, UnicodeDecodeError):
            return []  # unreadable or not text: not our business

    hits = []
    for lineno, line in enumerate(lines, 1):
        for col, ch in enumerate(line, 1):
            if is_forbidden(ch):
                hits.append(f"{path}:{lineno}:{col}: {ch!r} ({describe(ch)})")
        for col, ch in escaped_violations(line):
            hits.append(
                f"{path}:{lineno}:{col}: escaped {ch!r} ({describe(ch)}) "
                f"-- an escape is still an emoji on screen"
            )
    return hits


def describe(ch: str) -> str:
    try:
        return unicodedata.name(ch)
    except ValueError:
        return f"U+{ord(ch):04X}"


def main(argv: list[str]) -> int:
    args = argv[1:]
    if "--help" in args or "-h" in args:
        print(__doc__)
        return 0

    if "--self-test" in args:
        return _self_test()

    staged = "--staged" in args
    if staged:
        paths = staged_files()
    elif args:
        paths = args
    else:
        paths = tracked_files()

    violations = []
    for path in paths:
        violations.extend(check(path, staged=staged))

    if violations:
        print("Emoji are a failure state. Found:\n", file=sys.stderr)
        for v in violations:
            print(f"  {v}", file=sys.stderr)
        print(
            f"\n{len(violations)} violation(s). "
            f"Permitted pictographs: {''.join(sorted(ALLOWED))}",
            file=sys.stderr,
        )
        return 1

    print(f"no-emoji: clean ({len(paths)} files checked)")
    return 0


def _self_test() -> int:
    """`--self-test`: prove the checker detects what it claims to.

    The fixtures are assembled at runtime from codepoint numbers. Writing them
    out would put the very characters this file rejects into this file, and the
    gate scans itself.
    """
    padlock_escape = "let s = \"\\u{%x} Stored\";" % 0x1F512
    circle_literal = "const X = \"%s\";" % chr(0x26AA)
    fullwidth_ok = "banner \\u{%x} is fine" % 0xFF41

    cases = [
        ("plain ascii", False),
        ("arrow %s is allowed" % chr(0x2192), False),
        ("check %s is allowed" % chr(0x2713), False),
        (padlock_escape, True),
        (circle_literal, True),
        (fullwidth_ok, False),
    ]

    failures = 0
    for text, should_flag in cases:
        flagged = bool(escaped_violations(text)) or any(is_forbidden(c) for c in text)
        if flagged != should_flag:
            print(f"  self-test FAILED: {text!r} -> {flagged}, want {should_flag}")
            failures += 1
    print("self-test: clean" if not failures else f"self-test: {failures} failure(s)")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))

