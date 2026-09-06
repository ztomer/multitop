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
CHAR_RANGE = re.compile(r"'([a-zA-Z0-9])'\.\.='([a-zA-Z0-9])'")

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
            # Ranges bind every character between, not just the endpoints:
            # `c @ '1'..='9'` matched '1' and '9' and called '5' unbound.
            for lo, hi in CHAR_RANGE.findall(arm):
                for code in range(ord(lo), ord(hi) + 1):
                    keys.add(chr(code).lower())
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
    found.extend(help_table_offenders(root, keys))
    return found


HELP_TABLE = re.compile(r"const HELP_ROWS.*?\[(.*?)\];", re.DOTALL)
HELP_ENTRY = re.compile(r'\(\s*"([^"]+)"\s*,')
HELP_RANGE = re.compile(r"^([0-9])-([0-9])$")


def help_table_offenders(
    root: Path, keys: set[str]
) -> list[tuple[Path, int, str, str]]:
    """Every key the help overlay names must be bound.

    The prose patterns above cannot see the `("q", "...")` table the help
    panel is built from -- bare key literals match none of those shapes --
    so a table entry naming a dead key would ship silently. This reads the
    table instead of the prose.
    """
    found = []
    modals = root / "modals.rs"
    text = modals.read_text(encoding="utf-8") if modals.is_file() else ""
    table = HELP_TABLE.search(text)
    if table is None:
        # Fail loud, not silent: if the table moved or was renamed, this
        # check stopping quietly is exactly the drift it exists to prevent.
        return [(modals, 0, "?", "HELP_ROWS not found -- the help table moved?")]
    for key in HELP_ENTRY.findall(table.group(1)):
        names: list[str] = []
        if (m := HELP_RANGE.fullmatch(key)) is not None:
            names = [str(n) for n in range(int(m.group(1)), int(m.group(2)) + 1)]
        elif len(key) == 1:
            names = [key]
        else:
            found.append((modals, 0, key, f"help key {key!r} is not a single key"))
            continue
        for name in names:
            if name.lower() not in keys:
                found.append((modals, 0, name, f"help names {name!r}, nothing binds it"))
    return found


def self_test() -> int:
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        (root / "run.rs").write_text("match key { KeyCode::Char('e' | 'E') => open(), }", encoding="utf-8")
        (root / "modals.rs").write_text(
            'const HELP_ROWS: &[(&str, &str)] = &[("e", "edit")];', encoding="utf-8"
        )
        (root / "bad.rs").write_text('let s = "press q to save";', encoding="utf-8")
        hits = offenders(root)
        if not any(h[2] == "q" for h in hits):
            print("self-test FAILED: an unbound key hint was not detected")
            return 1

        (root / "bad.rs").write_text('let s = "press e to save";', encoding="utf-8")
        if offenders(root):
            print("self-test FAILED: a bound key hint was reported")
            return 1

        # Ranges bind their middles, not just their endpoints.
        (root / "run.rs").write_text(
            "match key { KeyCode::Char(c @ '1'..='9') => sel(c), }", encoding="utf-8"
        )
        if "5" not in bound_keys(root):
            print("self-test FAILED: a range-bound key reads as unbound")
            return 1

        # The help table is read as a table: an entry naming a dead key fails,
        # and a moved table fails loud instead of stopping silently.
        (root / "run.rs").write_text(
            "match key { KeyCode::Char('q') => quit(), }", encoding="utf-8"
        )
        (root / "bad.rs").write_text("let ok = 1;", encoding="utf-8")
        (root / "modals.rs").write_text(
            'const HELP_ROWS: &[(&str, &str)] = &[("q", "quit"), ("1-9", "select")];',
            encoding="utf-8",
        )
        hits = offenders(root)
        if not any(h[2] == "2" for h in hits):
            print("self-test FAILED: a dead help-table key was not detected")
            return 1
        (root / "modals.rs").write_text("// no table here", encoding="utf-8")
        hits = offenders(root)
        if len(hits) != 1 or hits[0][2] != "?":
            print("self-test FAILED: a missing help table was not reported")
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
