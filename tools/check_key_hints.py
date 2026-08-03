#!/usr/bin/env python3
"""Fail if a user-facing string tells the operator to press a key nothing binds.

Three help lines named two keys -- `p` and `o` -- that were bound to nothing:

    "will prompt · p to save"
    "Set a password for this host with o in Settings"
    "set those again with p."

The real key is `e`, which two other lines got right, so the program named three
different keys for one action and two of them did nothing when pressed. This is
the worst shape a documentation lie can take: every one of those lines appears
at the exact moment the operator is stuck and needs the instruction to work on
the first try.

The rule: if a string the user reads names a key, that key must be a live arm of
a `KeyCode::Char(..)` match somewhere in the crate.

Run with --self-test to check the checker.
"""

from __future__ import annotations

import re
import sys
import tempfile
from pathlib import Path

SRC = Path("crates/multitop/src")

# `KeyCode::Char('e' | 'E')`, `KeyCode::Char('y' | 'Y')`, `KeyCode::Char(c @ '1'..='9')`
BINDING = re.compile(r"KeyCode::Char\(([^)]*)\)")
CHAR_LIT = re.compile(r"'(.)'")

# What a key hint looks like in prose the user reads. Deliberately narrow: these
# are the shapes that have actually appeared, and a scanner that guesses more
# widely produces false alarms nobody will keep fixing.
HINTS = [
    # "press e", "with e in Settings", "e to save", "settings ('e')"
    re.compile(r"\bpress ([a-zA-Z]) \b"),
    re.compile(r"\bwith ([a-zA-Z]) in\b"),
    re.compile(r"\bwith ([a-zA-Z])\.\B"),
    re.compile(r"\b([a-zA-Z]) to (?:save|run|go back|quit|cancel|confirm)\b"),
    re.compile(r"\('([a-zA-Z])'\)"),
]

# Keys the runtime binds but that no `KeyCode::Char` arm mentions, because they
# are handled structurally. Keep this list short and justified.
ALWAYS_BOUND = {
    "u",  # the upgrade modal's confirm, matched as Char('u' | 'U' | 'y' | 'Y')
}


def bound_keys(root: Path) -> set[str]:
    """Every character a `KeyCode::Char` arm can match, lowercased."""
    keys: set[str] = set(ALWAYS_BOUND)
    for path in root.rglob("*.rs"):
        for arm in BINDING.findall(path.read_text(encoding="utf-8")):
            for ch in CHAR_LIT.findall(arm):
                keys.add(ch.lower())
    return keys


def string_literals(text: str) -> list[tuple[int, str]]:
    """Double-quoted literals with their 1-based line numbers."""
    out = []
    for n, line in enumerate(text.splitlines(), start=1):
        for lit in re.findall(r'"((?:[^"\\]|\\.)*)"', line):
            out.append((n, lit))
    return out


def offenders(root: Path) -> list[tuple[Path, int, str, str]]:
    keys = bound_keys(root)
    found = []
    for path in sorted(root.rglob("*.rs")):
        text = path.read_text(encoding="utf-8")
        for lineno, lit in string_literals(text):
            for pattern in HINTS:
                for key in pattern.findall(lit):
                    if key.lower() not in keys:
                        found.append((path, lineno, key, lit))
    return found


def self_test() -> int:
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        (root / "run.rs").write_text("match key { KeyCode::Char('e' | 'E') => open(), }", encoding="utf-8")
        (root / "bad.rs").write_text('let s = "press q to save";', encoding="utf-8")
        hits = offenders(root)
        if not any(h[2] == "q" for h in hits):
            print("self-test FAILED: an unbound key hint was not detected")
            return 1

        (root / "bad.rs").write_text('let s = "press e to save";', encoding="utf-8")
        if offenders(root):
            print("self-test FAILED: a bound key hint was reported")
            return 1
    print("check_key_hints self-test: ok")
    return 0


def main() -> int:
    if "--self-test" in sys.argv:
        return self_test()
    if not SRC.is_dir():
        print(f"key-hints: {SRC} not found -- run from the repository root")
        return 1
    hits = offenders(SRC)
    if not hits:
        print("key-hints: clean")
        return 0
    print("key-hints: a user-facing string names a key nothing binds\n")
    for path, lineno, key, lit in hits:
        print(f"  {path}:{lineno}: {key!r} is not bound")
        print(f"    {lit[:100]}")
    print(
        "\nUse the constant for the key, not a letter typed from memory --\n"
        "`consts::SETTINGS_KEY` is the one that exists. A hint that names a\n"
        "dead key is read at the moment the operator is already stuck."
    )
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
