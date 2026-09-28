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
import tempfile
import time
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent

#: The bar. The same WCAG AA figure the palette module uses, and named here so a
#: change to one and not the other is visible.
MIN_CONTRAST = 4.5

#: The two arms: the environment that stands in for the terminal's report, and
#: the colour that report is measured against.
ARMS = {
    "light": ("0;15", (255, 255, 255), "a light terminal"),
    "dark": (None, (30, 30, 30), "this machine's own dark terminal"),
}

#: `ESC [ 38 ; 2 ; R ; G ; B m`
COLOUR = re.compile(r"\x1b\[38;2;(\d+);(\d+);(\d+)m")

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
    subprocess.run(["tmux", "kill-session", "-t", session], capture_output=True)
    env = "" if colorfgbg is None else f'COLORFGBG="{colorfgbg}" '
    subprocess.run(
        [
            "tmux", "new-session", "-d", "-s", session,
            "-x", str(SIZE[0]), "-y", str(SIZE[1]),
            "sh", "-c", f"{env}{binary} --local; printf '\\nEXITED rc=%s\\n' \"$?\"; sleep 300",
        ],
        check=True,
        capture_output=True,
        timeout=30,
    )
    try:
        time.sleep(SETTLE_SECONDS)
        out = subprocess.run(
            ["tmux", "capture-pane", "-p", "-e", "-t", session],
            capture_output=True, text=True, timeout=30, check=True,
        ).stdout
    finally:
        subprocess.run(["tmux", "kill-session", "-t", session], capture_output=True)
    return out


def plain(panes: str) -> str:
    """The pane's text with every escape removed -- for substring tests only.

    Never for measurement: the colours ARE the escapes, and stripping them is
    the whole reason the first version of this check could not see the defect it
    was written to find.
    """
    return ESCAPE.sub("", panes)


def colours_in(panes: str) -> collections.Counter:
    return collections.Counter(
        (int(r), int(g), int(b)) for r, g, b in COLOUR.findall(panes)
    )


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--binary", type=Path, default=REPO / "target" / "release" / "multitop",
        help="the built binary to run (default: target/release/multitop)",
    )
    args = parser.parse_args()
    if not args.binary.is_file():
        print(f"light-terminal-check: {args.binary} does not exist -- build it "
              f"first (./build.sh)")
        return 1

    seen: dict[str, set[tuple[int, int, int]]] = {}
    failures: list[str] = []

    for arm, (colorfgbg, background, description) in ARMS.items():
        panes = capture(args.binary, colorfgbg, f"lightcheck-{arm}")
        if panes is None:
            return 1
        if READY not in ESCAPE.sub("", panes):
            failures.append(
                f"{arm}: the app never drew a frame (no {READY!r} in the pane) -- "
                f"it did not start, or it exited"
            )
            print("    " + " | ".join(panes.strip().splitlines()[:4]))
            continue
        used = colours_in(panes)
        seen[arm] = set(used)
        print(f"  {arm:<6} ({description}) — {len(used)} colours:")
        for rgb, count in used.most_common():
            ratio = contrast(rgb, background)
            mark = "   <-- under the bar" if ratio < MIN_CONTRAST else ""
            print(f"      rgb{rgb!s:<18} {ratio:>6.2f}:1{mark}")
            if ratio < MIN_CONTRAST:
                failures.append(
                    f"{arm}: rgb{rgb} is {ratio:.2f}:1 against "
                    f"{background}, under {MIN_CONTRAST}"
                )

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
    print(f"  ✓ every rendered colour clears {MIN_CONTRAST}:1 on both arms, and "
          f"the two palettes differ")
    return 0


if __name__ == "__main__":
    sys.exit(main())
