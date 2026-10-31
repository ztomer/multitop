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

# Named colours (2026-10-10)

The 24-bit literal was half the class. The command bar shipped white-on-white
in every theme from v0.50.0: its text was `Color::White` and `Color::DarkGray`
-- ratatui NAMED colours, which the terminal resolves from its own 16-colour
table, so no palette can promise they read and no adaptation ever sees them.
This checker matched only `38;2`, so all 51 such sites in the chrome passed it.
`Color::<Named>` and `Color::Indexed` are now the same finding as a literal.
The one legitimate site is `ansi.rs`, which maps a TOOL's own SGR codes (`31`
is red) to ratatui: the tool chose that colour, not this app.

# Fixed palettes (2026-10-10)

The third form of the class is a whole palette chosen at the call site:
`color::ANSI` or `THEMES[0]`, one theme's raw values, never adapted. The help
modal, the vault wait, the panel placeholders and every task's error and status
line drew that way whatever theme was on; on Nord with no answer from the
terminal, "connecting..." was 2.65:1. In the client the current palette is
`App::current_theme()` or, where no `App` reaches, `palette_store::current()`,
and `palette_store.rs` is the one file that may name a theme by index.

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
    "crates/agent/src/palette_view.rs": "writes escapes: it builds the adapted escapes the render path reads",
}

#: Files allowed to name a terminal colour, each with the reason.
NAMED_ALLOWED = {
    "crates/multitop/src/ansi.rs": "maps a tool's own SGR codes to ratatui -- the "
    "tool chose those colours, not the theme",
    "crates/multitop/src/fetch_render.rs": "the fetch view's swatch row shows the "
    "TERMINAL's eight colours, as every fetch tool does -- they are the subject, "
    "not the ink",
}

#: A ratatui named or indexed colour: the terminal decides what it looks like.
NAMED = re.compile(
    r"Color::(?:Black|Red|Green|Yellow|Blue|Magenta|Cyan|Gray|DarkGray|LightRed|"
    r"LightGreen|LightYellow|LightBlue|LightMagenta|LightCyan|White|Indexed)\b"
)

#: A palette chosen at the call site rather than the one the app is drawing with.
#: Client-only: the agent's standalone mode chooses its theme from the
#: environment, which is a theme choice and not a bypass of one.
FIXED = re.compile(r"color::ANSI\b|THEMES\[\d+\]")
FIXED_SCOPE = "crates/multitop/src/"
FIXED_ALLOWED = {
    "crates/multitop/src/palette_store.rs": "builds the default palette and publishes the current one",
}

#: The floor on files read. Not a tuning knob: the render crates are the app, and
#: a check that has read none of them has established nothing. One is the minimum
#: that proves the walk works; the real tree is far above it.
MIN_FILES = 1

#: The thing being hunted: `ESC [ 38 ; 2 ; R ; G ; B m`, with the digits bounded
#: so a `\x1b[38;2;` prefix in prose is not a match.
LITERAL = re.compile(r"\\x1b\[38;2;\d+;\d+;\d+m|38;2;\d+;\d+;\d+m")

#: A terminal-decided colour written as an escape: a 16-colour SGR (`31`, `90`,
#: `44`, `36;1`) or a 256-colour one (`38;5;N`). The escape form of `NAMED`.
#: The refit header and rule were `ESC[36;1m` and `ESC[90m`, and three sudo
#: tips `ESC[33m`, and the 24-bit pattern above passed all of them.
SGR_NAMED = re.compile(r"\\x1b\[(?:\d+;)*(?:3[0-7]|9[0-7]|4[0-7]|10[0-7]|[34]8;5;\d+)(?:;\d+)*m")


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
                if (
                    LITERAL.search(line)
                    or (rel not in NAMED_ALLOWED and (NAMED.search(line.split("//", 1)[0]) or SGR_NAMED.search(line)))
                    or (
                        rel.startswith(FIXED_SCOPE)
                        and rel not in FIXED_ALLOWED
                        and FIXED.search(line.split("//", 1)[0])
                    )
                ):
                    hits.append((rel, lineno, line.strip()[:100]))
    return hits


def self_test() -> int:
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        src = root / "crates" / "multitop" / "src"
        src.mkdir(parents=True)
        (root / "crates" / "agent" / "src").mkdir(parents=True)

        (src / "draw.rs").write_text('let a = format!("\\x1b[38;2;255;85;85m{}", bold);\n', encoding="utf-8")
        if not offenders(root):
            print("self-test FAILED: a hard-coded 24-bit colour was not detected")
            return 1

        # What must NOT be flagged: the palette itself, a test, and terminal
        # state escapes that are not colours.
        (root / "crates" / "agent" / "src" / "color.rs").write_text('red: "\\x1b[38;2;255;85;85m",\n', encoding="utf-8")
        (src / "draw_tests.rs").write_text('assert_eq!(x, "\\x1b[38;2;255;85;85m");\n', encoding="utf-8")
        (src / "draw.rs").write_text('let reset = "\\x1b[0m";\nlet default = "\\x1b[39m";\n', encoding="utf-8")
        if offenders(root):
            print("self-test FAILED: the palette, a test module, or a non-colour escape was reported")
            return 1

        # The escape the palette API hands back, spelled in the docs of a call
        # site, must not be a hit either -- it is `meter_high()`, not a literal.
        (src / "draw.rs").write_text(
            '// was: ESC[38;2;255;85;85m, now theme.meter_high()\nlet a = format!("{}{}", theme.meter_high(), bold);\n',
            encoding="utf-8",
        )
        hits = offenders(root)
        # The comment IS a literal, and flagging it is correct: a comment that
        # records the old value is how the value comes back.
        if not hits:
            print(
                "self-test FAILED: a literal quoted in a comment was not "
                "flagged -- that comment is how the literal returns"
            )
            return 1

    # Named colours: flagged in the chrome, allowed in the SGR mapper, and not
    # flagged in a comment that merely mentions one.
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        src = root / "crates" / "multitop" / "src"
        src.mkdir(parents=True)
        (root / "crates" / "agent" / "src").mkdir(parents=True)
        for case in (
            'format!(" \\x1b[90m{}\\x1b[0m", rule)',
            'format!("\\x1b[36;1m{title}\\x1b[0m")',
            '"\\x1b[33m\\u{2192} Tip\\x1b[0m"',
            '"\\x1b[38;5;208mtext"',
            "let label = Style::default().fg(Color::DarkGray);",
            "Span::styled(k, Style::default().fg(ratatui::style::Color::White))",
            "let c = Color::Indexed(8);",
        ):
            (src / "keybar.rs").write_text(case + "\n", encoding="utf-8")
            if not offenders(root):
                print(f"self-test FAILED: a named colour was not detected: {case}")
                return 1
        (src / "keybar.rs").write_text(
            "let c = Color::Rgb(1, 2, 3); // not Color::White any more\n"
            'let r = "\\x1b[0m"; let b = "\\x1b[1m"; let d = "\\x1b[2m"; let f = "\\x1b[39m";\n',
            encoding="utf-8",
        )
        (src / "ansi.rs").write_text("31 => style = style.fg(Color::Red),\n", encoding="utf-8")
        if offenders(root):
            print("self-test FAILED: an Rgb colour, a comment, or ansi.rs was reported")
            return 1

    # Fixed palettes: flagged in the client, allowed in the store and in a
    # comment, not policed in the agent.
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        src = root / "crates" / "multitop" / "src"
        src.mkdir(parents=True)
        (root / "crates" / "agent" / "src").mkdir(parents=True)
        for case in ("let pal = &multitop_agent::color::ANSI;", "let theme = &multitop_agent::color::THEMES[0];"):
            (src / "panel.rs").write_text(case + "\n", encoding="utf-8")
            if not offenders(root):
                print(f"self-test FAILED: a fixed palette was not detected: {case}")
                return 1
        (src / "panel.rs").write_text(
            "let pal = crate::palette_store::current(); // was color::ANSI\n",
            encoding="utf-8",
        )
        (src / "palette_store.rs").write_text("&THEMES[0],\n", encoding="utf-8")
        (root / "crates" / "agent" / "src" / "lib.rs").write_text("&color::ANSI\n", encoding="utf-8")
        if offenders(root):
            print("self-test FAILED: the store, a comment, or the agent was reported")
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
        print("no-hard-coded-colour: none of the rendering crates were found -- run from the repository root")
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
            f"render path. A 24-bit literal, a named terminal colour, or a fixed "
            f"palette (color::ANSI, THEMES[n]) does not go through PaletteView, "
            f"so it is never adapted -- invisible on one terminal, unreadable on "
            f"another. Use a role of App::current_theme() (or "
            f"palette_store::current() where no App reaches, or a Chrome role in "
            f"ratatui chrome), and if the colour genuinely is not a role, say why "
            f"in ALLOWED."
        )
        return 1
    print(f"no-hard-coded-colour: ok ({len(present)} crates, {count} files read, {len(ALLOWED)} allowed)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
