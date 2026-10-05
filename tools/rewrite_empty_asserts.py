#!/usr/bin/env python3
"""Rewrite `assert!(x.is_empty())` into `multitop_testassert::assert_empty!(x)`.

The migration that cleared this workspace's 115 emptiness assertions in one
commit (2026-10-04), kept because the same shape is in other repos in the
estate and re-deriving the transformation is worse than reading it.

It edits SOURCE, so it prints what it would do and changes nothing unless
`--apply` is passed. It is idempotent: a second run finds nothing, because the
form it rewrites is the one it removes.

    python3 tools/rewrite_empty_asserts.py            # print the rewrites
    python3 tools/rewrite_empty_asserts.py --apply    # make them

What it does NOT touch, and why:

*   An `assert!` whose message interpolates (`{x:?}`). That is the shape
    gates_of_heck's `check_no_empty_assert.py` exempts on purpose -- "already
    made" -- and it is what `crates/testassert`'s own macro bodies use.
    Rewriting them would make the helper call itself.
*   A compound condition (`&&`, `||`) or anything that is not an emptiness
    test: `assert!(rows.len() >= 3)` is not this pattern.
"""

from __future__ import annotations

import argparse
import os
import re
import sys
from pathlib import Path

sys.path.insert(
    0, os.path.join(os.path.expanduser("~"), "Projects", "gates_of_heck", "checks")
)
import check_no_empty_assert as C  # noqa: E402

REPO = Path(__file__).resolve().parent.parent
MACRO_PATH = "multitop_testassert"

ASSERT_MACRO = re.compile(r"(?<![\w:])(?:debug_)?assert!\s*\(")


def masked_lines(text: str) -> list[str]:
    """One entry per line, SAME LENGTH as the input, comments blanked to spaces.

    Same length is the whole point: an offset into this string is an offset into
    the real line, so a finding can be located in the source it came from.
    `code_portions` drops comment characters instead, which desynchronises any
    offset built from it.
    """
    out: list[str] = []
    depth = 0
    for line in text.split("\n"):
        buf: list[str] = []
        i = 0
        while i < len(line):
            if line.startswith("/*", i):
                depth += 1
                buf.append("  ")
                i += 2
                continue
            if depth > 0 and line.startswith("*/", i):
                depth -= 1
                buf.append("  ")
                i += 2
                continue
            if depth == 0 and line.startswith("//", i):
                buf.append(" " * (len(line) - i))
                break
            buf.append(" " if depth > 0 else line[i])
            i += 1
        out.append("".join(buf))
    return out


def _string_end(text: str, i: int) -> int:
    """Index just past the string literal starting at `text[i] == '"'`."""
    i += 1
    while i < len(text):
        if text[i] == "\\":
            i += 2
            continue
        if text[i] == '"':
            return i + 1
        i += 1
    return i


def close_paren(text: str, open_at: int) -> int:
    """Index of the `)` matching the `(` at `open_at`, skipping literals."""
    depth = 0
    i = open_at
    while i < len(text):
        ch = text[i]
        if ch == '"':
            i = _string_end(text, i)
            continue
        if ch == "(":
            depth += 1
        elif ch == ")":
            depth -= 1
            if depth == 0:
                return i
        i += 1
    raise ValueError("unbalanced parentheses")


def split_args(body: str) -> list[str]:
    """Top-level arguments of a macro body, string literals respected."""
    args: list[str] = []
    depth = 0
    start = 0
    i = 0
    while i < len(body):
        ch = body[i]
        if ch == '"':
            i = _string_end(body, i)
            continue
        if ch in "([":
            depth += 1
        elif ch in ")]":
            depth -= 1
        elif ch == "," and depth == 0:
            args.append(body[start:i])
            start = i + 1
        i += 1
    tail = body[start:]
    if tail.strip():
        args.append(tail)
    return [a.strip() for a in args]


IS_EMPTY = re.compile(r"^(?P<value>.+)\.is_empty\s*\(\s*\)$", re.S)
LEN_CMP = re.compile(r"^(?P<value>.+)\.len\s*\(\s*\)\s*(?P<op>==|!=|<|>)\s*0$", re.S)
LEN_EQ = re.compile(r"^0\s*(?P<op>==|!=)\s*(?P<value>.+)\.len\s*\(\s*\)$", re.S)


def rewrite(condition: str, message: str | None) -> str | None:
    """The replacement for one invocation, or None when the shape is unknown."""
    cond = condition.strip()
    if "&&" in cond or "||" in cond:
        return None
    negated = cond.startswith("!")
    body = cond[1:].strip() if negated else cond

    match = IS_EMPTY.match(body)
    if match:
        value, want_empty = match.group("value").strip(), not negated
    else:
        match = LEN_CMP.match(body) or LEN_EQ.match(body)
        if not match:
            return None
        op = match.group("op")
        # `len() > 0` is the negated spelling; the others are direct.
        want_empty = op in ("==", "<") if match.re is LEN_CMP else op == "=="
        value = match.group("value").strip()

    if not value:
        return None
    name = "assert_empty" if want_empty else "assert_not_empty"
    why = message.strip() if message is not None else ""
    # An empty message carries nothing; `assert!(x.is_empty(), "")` was a
    # placeholder for one and should not survive as `assert_empty!(x, "")`.
    tail = f", {why}" if why and why not in ('""', 'r""') else ""
    return f"{MACRO_PATH}::{name}!({value}{tail})"


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument(
        "--apply", action="store_true", help="write the rewrites (default: print only)"
    )
    args = ap.parse_args()

    root = C.repo_root()
    files = C._files(root, False, None)
    total = 0
    skipped: list[str] = []

    for rel in files:
        blob = C.content_bytes(root, rel, staged=False)
        if blob is None or C.is_generated_blob(blob):
            continue
        text = blob.decode("utf-8")
        masked = masked_lines(text)
        offsets: list[int] = []
        acc = 0
        for line in text.split("\n"):
            offsets.append(acc)
            acc += len(line) + 1

        # Collect every replacement first, then apply back to front so earlier
        # offsets stay valid.
        edits: list[tuple[int, int, str]] = []
        for lineno, line in enumerate(masked):
            for m in ASSERT_MACRO.finditer(line):
                start = offsets[lineno] + m.start()
                body_open = text.index("(", start)
                end = close_paren(text, body_open)
                invocation = text[start : end + 1]
                inner = invocation[invocation.index("(") + 1 : -1]
                pieces = split_args(inner)
                condition = pieces[0] if pieces else ""
                message = pieces[1] if len(pieces) > 1 else None
                # The macro BODIES in crates/testassert assert emptiness with an
                # interpolating message, which is the shape the house checker
                # exempts on purpose ("already made"). Rewriting them would make
                # the helper call itself, so they are left exactly as they are.
                if message is not None and "{" in message and "}" in message:
                    continue
                # clippy counts only the message-less form; the house checker
                # counts both, and this rewrite must clear both, so the message
                # is carried across rather than dropped.
                replacement = rewrite(condition, message)
                if replacement is None:
                    skipped.append(f"{rel}:{lineno + 1}: {condition}")
                    continue
                edits.append((start, end + 1, replacement))

        if not edits:
            continue
        for start, end, replacement in sorted(edits, reverse=True):
            text = text[:start] + replacement + text[end:]
        total += len(edits)
        for start, _end, replacement in sorted(edits):
            lineno = text.count("\n", 0, start) + 1
            print(f"{rel}:{lineno}\n    {replacement}")
        if args.apply:
            (REPO / rel).write_text(text)

    print(f"\n{total} rewritten, {len(skipped)} skipped", file=sys.stderr)
    for s in skipped:
        print(f"  SKIPPED {s}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())