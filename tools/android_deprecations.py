#!/usr/bin/env python3
"""The Android build's deprecations, as a ratchet.

    android_deprecations.py <gradle log> <ratchet file>
    android_deprecations.py --self-test

Every line of the Gradle log that says "deprecated" must be matched by an entry
of the ratchet file, and every entry must match a line of the log. An entry is
one line, `<substring> | <reason>`; blank lines and `#` comments are ignored.

Why a ratchet and not `org.gradle.warning.mode=fail`: the build's own code can
be held to zero, but a deprecation raised INSIDE a plugin (AGP 9.4.1 calls
Gradle's deprecated Configuration.setVisible) cannot be fixed here. Failing on
it would block every build until a release; ignoring all of them would hide
ours. Holding each one by name, with its reason, does neither -- and an entry
whose deprecation stopped firing (the plugin moved on) fails until it is
deleted, so the held set only shrinks.

Exit 0 when the log and the file agree, 1 when they do not, 2 on bad input.
"""

from __future__ import annotations

import sys
from pathlib import Path


def entries(text: str) -> list[tuple[str, str]]:
    out = []
    for raw in text.splitlines():
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        key, sep, reason = line.partition(" | ")
        if not sep or not key.strip() or not reason.strip():
            raise ValueError(f"entry without a reason: {raw!r} (want '<substring> | <reason>')")
        out.append((key.strip(), reason.strip()))
    return out


def findings(log: str) -> list[str]:
    return sorted({l.strip() for l in log.splitlines() if "deprecated" in l.lower()})


def problems(log: str, held: list[tuple[str, str]]) -> list[str]:
    found = findings(log)
    out = [f"new deprecation (fix it, or hold it with a reason): {f}"
           for f in found if not any(k in f for k, _ in held)]
    out += [f"stale entry, no longer printed -- delete it: {k}"
            for k, _ in held if not any(k in f for f in found)]
    return out


def self_test() -> int:
    held = [("setVisible", "AGP 9.4.1")]
    if problems("The Configuration.setVisible(boolean) method has been deprecated.\n", held):
        print("self-test: a held deprecation was reported", file=sys.stderr)
        return 1
    if not problems("x has been deprecated\nThe setVisible method has been deprecated\n", held):
        print("self-test: a new deprecation was NOT reported", file=sys.stderr)
        return 1
    if not problems("BUILD SUCCESSFUL\n", held):
        print("self-test: a stale entry was NOT reported", file=sys.stderr)
        return 1
    try:
        entries("setVisible\n")
    except ValueError:
        return 0
    print("self-test: an entry without a reason was accepted", file=sys.stderr)
    return 1


def main(argv: list[str]) -> int:
    if argv[1:] == ["--self-test"]:
        return self_test()
    if len(argv) != 3:
        print(__doc__, file=sys.stderr)
        return 2
    try:
        held = entries(Path(argv[2]).read_text(encoding="utf-8"))
    except (OSError, ValueError) as e:
        print(f"android_deprecations: {e}", file=sys.stderr)
        return 2
    found = problems(Path(argv[1]).read_text(encoding="utf-8", errors="replace"), held)
    for p in found:
        print(f"  {p}", file=sys.stderr)
    return 1 if found else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
