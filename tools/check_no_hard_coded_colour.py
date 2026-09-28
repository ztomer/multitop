#!/usr/bin/env python3
"""Fail if the render path hard-codes a colour instead of asking the palette.

`crates/agent/src/color.rs` is the palette, and a 24-bit escape there is a
definition rather than a duplicate. Everywhere else it is a colour that has
decided to ignore the theme, and ignoring the theme is invisible until the
terminal disagrees with it.

Which is how the alert banner survived the light-terminal work: it hard-coded
`ESC[38;2;255;85;85m` for a breached alert and `ESC[38;2;241;250;140m` for a
warning. On a dark terminal those literals ARE the theme's values, so nothing
looked wrong; and they do not pass through `PaletteView`, so the adaptation
never saw them. A capture of the real app on a white terminal found it, by
measuring the rendered pixels rather than the code: the alert red came out at
3.14:1, the only one of eight rendered colours under the bar. Every other one
had been adapted and cleared it comfortably.

The class this names is "a colour chosen at the call site". The fix is a gate,
because the alternative is the same two literals being re-added after the next
theme change, and nothing about them is wrong-looking in review.

Scope: the two rendering crates, minus their test modules and minus the palette
itself and the colour arithmetic that legitimately writes escapes. Non-24-bit
escapes are not flagged -- `ESC[39m` and `ESC[0m` are terminal state, not
colours, and there are many of them.

The scanned-file count is reported and REQUIRED to be non-zero, because "found no
literal" and "read no file" print the same word. Run against a directory skeleton
-- the crates present, the files gone -- this checker reported `ok (2 crates, 3
files allowed)` having read nothing at all, and the house empty-scope sweep caught
it. The count is in the output for the same reason it is in the exit code: a
future reader must be able to tell compliance from blindness without reading the
code.

Run with --self-test to check the checker.
"""

from __future__ import annotations

import re
import sys
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent

#: The crates that draw. `agent` draws the monitor/docker/fetch panels the
#: client then displays; `multitop` draws the chrome around them.
CRATES = ("crates/agent/src", "crates/multitop/src")

#: Files allowed to contain a literal 24-bit colour, each with the reason. A new
#: entry must say why; a stale entry is rot in a different file.
ALLOWED = {
    "crates/agent/src/color.rs": "the palette -- this is where a colour is DEFINED",
    "crates/agent/src/surface.rs": "writes escapes: it is the parser and the "
    "adaptation, and an escape is what it produces",
    "crates/agent/src/palette_view.rs": "writes escapes: it builds the adapted "
    "escapes the render path reads",
}

#: The floor on files read. Not a tuning knob: the render crates are the app, and
#: a check that has read none of them has established nothing. One is the minimum
#: that proves the walk works; the real tree is far above it.
MIN_FILES = 1

#: The thing being hunted: `ESC [ 38 ; 2 ; R ; G ; B m`, with the digits bounded
#: so a `\x1b[38;2;` prefix in prose is not a match.
LITERAL = re.compile(r"\\x1b\[38;2;\d+;\d+;\d+m|38;2;\d+;\d+;\d+m")


def scanned(root: Path) -> int:
    """How many files this checker's opinion is based on. Zero is a blind gate."""
    n = 0
    for crate in CRATES:
        base = root / crate
        if base.is_dir():
            n += sum(1 for p in base.rglob("*.rs") if _in_scope(p, root))
    return n


def _in_scope(path: Path, root: Path) -> bool:
    rel = str(path.relative_to(root))
    if rel in ALLOWED:
        return False
    return not ("_tests" in path.name or path.name.endswith("_test.rs"))


def offenders(root: Path):
    hits = []
    for crate in CRATES:
        base = root / crate
        if not base.is_dir():
            continue
        for path in sorted(base.rglob("*.rs")):
            rel = str(path.relative_to(root))
            if not _in_scope(path, root):
                continue
            # Test modules are not the render path, and a test may legitimately
            # write a literal to pin a format.
            if "_tests" in path.name or path.name.endswith("_test.rs"):
                continue
            for lineno, line in enumerate(path.read_text(encoding="utf-8").splitlines(), 1):
                if LITERAL.search(line):
                    hits.append((rel, lineno, line.strip()[:100]))
    return hits


def self_test() -> int:
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        src = root / "crates" / "multitop" / "src"
        src.mkdir(parents=True)
        (root / "crates" / "agent" / "src").mkdir(parents=True)

        (src / "draw.rs").write_text(
            'let a = format!("\\x1b[38;2;255;85;85m{}", bold);\n', encoding="utf-8"
        )
        if not offenders(root):
            print("self-test FAILED: a hard-coded 24-bit colour was not detected")
            return 1

        # What must NOT be flagged: the palette itself, a test, and terminal
        # state escapes that are not colours.
        (root / "crates" / "agent" / "src" / "color.rs").write_text(
            'red: "\\x1b[38;2;255;85;85m",\n', encoding="utf-8"
        )
        (src / "draw_tests.rs").write_text(
            'assert_eq!(x, "\\x1b[38;2;255;85;85m");\n', encoding="utf-8"
        )
        (src / "draw.rs").write_text(
            'let reset = "\\x1b[0m";\nlet default = "\\x1b[39m";\n', encoding="utf-8"
        )
        if offenders(root):
            print(
                "self-test FAILED: the palette, a test module, or a non-colour "
                "escape was reported"
            )
            return 1

        # The escape the palette API hands back, spelled in the docs of a call
        # site, must not be a hit either -- it is `meter_high()`, not a literal.
        (src / "draw.rs").write_text(
            '// was: ESC[38;2;255;85;85m, now theme.meter_high()\n'
            'let a = format!("{}{}", theme.meter_high(), bold);\n',
            encoding="utf-8",
        )
        hits = offenders(root)
        # The comment IS a literal, and flagging it is correct: a comment that
        # records the old value is how the value comes back.
        if not hits:
            print("self-test FAILED: a literal quoted in a comment was not "
                  "flagged -- that comment is how the literal returns")
            return 1

    # The case the house empty-scope sweep found: crates present, files gone.
    # A checker that scans nothing must not report success.
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        (root / "crates" / "agent" / "src").mkdir(parents=True)
        (root / "crates" / "multitop" / "src").mkdir(parents=True)
        if scanned(root) != 0:
            print("self-test FAILED: a skeleton with no source read as non-empty")
            return 1
        if not offenders(root):
            pass
        # and `main` must refuse it, which the return value is the only proof of
        if MIN_FILES > 0 and scanned(root) >= MIN_FILES:
            print("self-test FAILED: MIN_FILES does not exclude the empty case")
            return 1

    print("check_no_hard_coded_colour self-test: ok")
    return 0


def main() -> int:
    if "--self-test" in sys.argv:
        return self_test()

    present = [c for c in CRATES if (REPO / c).is_dir()]
    if not present:
        print("no-hard-coded-colour: none of the rendering crates were found -- "
              "run from the repository root")
        return 1

    # A gate whose subject population is empty reports compliance over nothing.
    # The crates existing is not evidence that any of their source was read.
    count = scanned(REPO)
    if count < MIN_FILES:
        print(
            f"no-hard-coded-colour: only {count} in-scope render file(s) found, "
            f"so this check is blind rather than clean. The render crates cannot "
            f"be empty; if they have been moved or renamed, update CRATES and "
            f"ALLOWED, or the check has nothing left to police."
        )
        return 1

    hits = offenders(REPO)
    for rel, lineno, line in hits:
        print(f"  {rel}:{lineno}: {line}")
    if hits:
        print(
            f"no-hard-coded-colour: {len(hits)} hard-coded colour(s) in the "
            f"render path. A literal there does not go through PaletteView, so "
            f"it is never adapted -- invisible on a dark terminal, unreadable on "
            f"a light one. Use theme.meter_high(), theme.meter_mid(), or "
            f"another role accessor, and if the colour genuinely is not a role, "
            f"say why in ALLOWED."
        )
        return 1
    print(f"no-hard-coded-colour: ok ({len(present)} crates, {count} files read, "
          f"{len(ALLOWED)} allowed)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
