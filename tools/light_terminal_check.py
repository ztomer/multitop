#!/usr/bin/env python3
"""Prove the palette is legible on a LIGHT terminal, on the real app.

The unit tests measure the arithmetic. This measures the ARTEFACT: it runs the
built binary in a real pty, sets the background the way a light terminal would
report it, captures what the app actually drew, and measures every colour that
came out against white.

That distinction earned its keep immediately. The arithmetic was entirely
correct -- 1380 tests, all green, every role clearing 4.5:1 in the palette -- and
the real app on a white terminal still drew the alert red at **3.14:1**, because
`ui/draw.rs` hard-coded that escape instead of asking the palette, so the
adaptation never saw it. Nothing in the suite could have found that: the escape
is correct for the dark terminal it was written on, and correct in the palette.

So the check is on the rendered bytes, and it has two arms because one arm
cannot tell "chose the right colours" from "always draws the same colours":

  light  COLORFGBG=0;15, measured against white
  dark   no COLORFGBG, measured against this machine's own background

Both must clear the bar, and the two palettes must DIFFER -- which is the
negative control. A check that passes because the app always draws dark-theme
colours would clear the light arm only by accident, and the difference assertion
is what catches that.

# Backgrounds (2026-10-10)

The first version measured each FOREGROUND against the terminal's background
and read only `38;2`. The command bar then shipped near-white on every dark
terminal: its fill was the adapted text colour and its labels were 16-colour
escapes, white on near-white at 1.02-1.73:1, and this check passed it -- the
bar's own background was in no measurement, and its labels were not a pattern
it read. It now walks the capture cell by cell, resolves each glyph's
background (`48;2`, else the terminal's), measures the glyph against THAT, and
fails any 16- or 256-colour escape outright: the terminal decides what those
look like, so nothing here can promise they read. Floors are the palette's
own: bold glyphs are text (4.5:1), the rest may be muted (3.0:1).

Usage:
    tools/light_terminal_check.py [--binary PATH]

Exit status 1 when a rendered colour is under the bar, when the two arms are
identical, or when the app did not draw at all.
"""

from __future__ import annotations

import argparse
import collections
import re
import shutil
import subprocess
import sys
import time
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent

#: The bars. The same figures as `adapt::MIN_TEXT_CONTRAST` and
#: `MIN_MUTED_CONTRAST`, named here so a change to one and not the other is
#: visible. Bold glyphs (keys, titles) are text; the rest may be de-emphasised.
MIN_CONTRAST = 4.5
MIN_MUTED_CONTRAST = 3.0

#: The two arms: the environment that stands in for the terminal's report, the
#: background that report is measured against, and the terminal's default
#: foreground (what a glyph with no colour of its own is drawn in).
ARMS = {
    "light": ("0;15", (255, 255, 255), (0, 0, 0), "a light terminal"),
    "dark": (None, (30, 30, 30), (229, 229, 229), "this machine's own dark terminal"),
}

#: An SGR sequence, or a run of text between escapes.
SGR_OR_TEXT = re.compile(r"\x1b\[([0-9;]*)m|\x1b\[[0-9;?]*[a-zA-Z]|([^\x1b]+)")

#: Ready marker: the keybar is drawn by the event loop, so its presence means the
#: loop is running and the frame is real. A pid is not readiness.
READY = "Quit"

#: Any escape sequence. The keybar styles each key individually -- on a light
#: terminal `Quit` arrives as `Q` in one colour and `uit` in another, with
#: escapes between the letters -- so a substring test against the RAW capture
#: fails on exactly the arm that matters. The first version of this check did
#: that and reported "the app never drew a frame" for both arms, while the app
#: was running and drawing.
ESCAPE = re.compile(r"\x1b\[[0-9;?]*[a-zA-Z]|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)")

SIZE = (200, 50)
SETTLE_SECONDS = 9


def linear(channel: int) -> float:
    value = channel / 255.0
    if value <= 0.04045:
        return value / 12.92
    return ((value + 0.055) / 1.055) ** 2.4


def luminance(rgb: tuple[int, int, int]) -> float:
    r, g, b = (linear(c) for c in rgb)
    return 0.2126 * r + 0.7152 * g + 0.0722 * b


def contrast(fg: tuple[int, int, int], bg: tuple[int, int, int]) -> float:
    a, b = luminance(fg), luminance(bg)
    high, low = max(a, b), min(a, b)
    return (high + 0.05) / (low + 0.05)


def capture(binary: Path, colorfgbg: str | None, session: str) -> str | None:
    """Run the app in a pty and return the pane's text WITH colour escapes."""
    if shutil.which("tmux") is None:
        print("light-terminal-check: tmux is not on PATH")
        return None
    subprocess.run(["tmux", "kill-session", "-t", session], capture_output=True, stdin=subprocess.DEVNULL, check=False)
    env = "" if colorfgbg is None else f'COLORFGBG="{colorfgbg}" '
    subprocess.run(
        [
            "tmux",
            "new-session",
            "-d",
            "-s",
            session,
            "-x",
            str(SIZE[0]),
            "-y",
            str(SIZE[1]),
            "sh",
            "-c",
            f"{env}{binary} --local; printf '\\nEXITED rc=%s\\n' \"$?\"; sleep 300",
        ],
        check=True,
        capture_output=True,
        timeout=30,
        stdin=subprocess.DEVNULL,
    )
    try:
        time.sleep(SETTLE_SECONDS)
        out = subprocess.run(
            ["tmux", "capture-pane", "-p", "-e", "-t", session],
            capture_output=True,
            text=True,
            timeout=30,
            check=True,
            stdin=subprocess.DEVNULL,
        ).stdout
    finally:
        subprocess.run(
            ["tmux", "kill-session", "-t", session], capture_output=True, stdin=subprocess.DEVNULL, check=False
        )
    return out


def plain(panes: str) -> str:
    """The pane's text with every escape removed -- for substring tests only.

    Never for measurement: the colours ARE the escapes, and stripping them is
    the whole reason the first version of this check could not see the defect it
    was written to find.
    """
    return ESCAPE.sub("", panes)


def cells(panes: str, default_fg, default_bg):
    """Every drawn glyph as `(fg, bg, bold)`, with colours resolved.

    A colour the terminal decides -- a 16- or 256-colour SGR -- comes back as
    the string naming it, never as an RGB this check would have to guess.
    """
    for line in panes.split("\n"):
        fg = bg = None
        bold = False
        for m in SGR_OR_TEXT.finditer(line):
            if m.group(2) is not None:
                for ch in m.group(2):
                    if not ch.isspace():
                        yield fg or default_fg, bg or default_bg, bold
                continue
            if m.group(1) is None:
                continue  # not SGR: movement, mode, nothing to colour
            params = [int(p) if p else 0 for p in m.group(1).split(";")]
            i = 0
            while i < len(params):
                p = params[i]
                if p == 0:
                    fg = bg = None
                    bold = False
                elif p == 1:
                    bold = True
                elif p == 22:
                    bold = False
                elif p == 39:
                    fg = None
                elif p == 49:
                    bg = None
                elif p in (38, 48) and params[i + 1 : i + 2] == [2] and i + 4 < len(params):
                    rgb = tuple(params[i + 2 : i + 5])
                    fg, bg = (rgb, bg) if p == 38 else (fg, rgb)
                    i += 4
                elif p in (38, 48) and params[i + 1 : i + 2] == [5]:
                    named = f"ESC[{p};5;{params[i + 2] if i + 2 < len(params) else '?'}m"
                    fg, bg = (named, bg) if p == 38 else (fg, named)
                    i += 2
                elif 30 <= p <= 37 or 90 <= p <= 97:
                    fg = f"ESC[{p}m"
                elif 40 <= p <= 47 or 100 <= p <= 107:
                    bg = f"ESC[{p}m"
                i += 1


def measure(panes: str, default_fg, default_bg) -> collections.Counter:
    """How many glyphs drew each (fg, bg, floor) pairing."""
    return collections.Counter(
        (fg, bg, MIN_CONTRAST if bold else MIN_MUTED_CONTRAST) for fg, bg, bold in cells(panes, default_fg, default_bg)
    )


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--binary",
        type=Path,
        default=REPO / "target" / "release" / "multitop",
        help="the built binary to run (default: target/release/multitop)",
    )
    args = parser.parse_args()
    if not args.binary.is_file():
        print(f"light-terminal-check: {args.binary} does not exist -- build it first (./build.sh)")
        return 1

    seen: dict[str, set] = {}
    failures: list[str] = []

    for arm, (colorfgbg, background, foreground, description) in ARMS.items():
        panes = capture(args.binary, colorfgbg, f"lightcheck-{arm}")
        if panes is None:
            return 1
        if READY not in ESCAPE.sub("", panes):
            failures.append(
                f"{arm}: the app never drew a frame (no {READY!r} in the pane) -- it did not start, or it exited"
            )
            print("    " + " | ".join(panes.strip().splitlines()[:4]))
            continue
        used = measure(panes, foreground, background)
        seen[arm] = {(fg, bg) for fg, bg, _ in used}
        print(f"  {arm:<6} ({description}) — {len(used)} ink/surface pairings, {sum(used.values())} glyphs:")
        for (fg, bg, floor), count in used.most_common():
            if isinstance(fg, str) or isinstance(bg, str):
                print(f"      {fg!s:<18} on {bg!s:<18} terminal-decided   <-- x{count}")
                failures.append(
                    f"{arm}: {count} glyph(s) in a terminal-decided colour "
                    f"({fg} on {bg}) -- no palette can promise it reads"
                )
                continue
            ratio = contrast(fg, bg)
            mark = f"   <-- under {floor}" if ratio < floor else ""
            print(f"      rgb{fg!s:<16} on rgb{bg!s:<16} {ratio:>6.2f}:1{mark}")
            if ratio < floor:
                failures.append(f"{arm}: {count} glyph(s) rgb{fg} on rgb{bg} at {ratio:.2f}:1, under {floor}")

    # The negative control. Two identical arms would mean the background is
    # being ignored, which passes every per-colour assertion above.
    if len(seen) == 2 and seen.get("light") and seen["light"] == seen.get("dark"):
        failures.append(
            "the light and dark arms drew IDENTICAL palettes, so the detected "
            "background is not reaching the render path"
        )

    if failures:
        print()
        for line in failures:
            print(f"  ✗ {line}")
        return 1
    print()
    print(
        f"  ✓ every rendered glyph clears its floor ({MIN_CONTRAST}:1 bold, "
        f"{MIN_MUTED_CONTRAST}:1 otherwise) against its own cell's background on "
        f"both arms, none is terminal-decided, and the two palettes differ"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
