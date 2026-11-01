"""A release installs its own build before it pushes.

The pre-push gate runs `tools/installed_freshness.py`, which refuses a push
when the installed `multitop` is not byte-identical to this build. A release's
bump commit changes the version the build embeds, so the binary installed
before the bump is stale by construction, and the push was refused: v0.67.5's
first push, 2026-11-01, with every other gate green. `release.sh` therefore
runs `./install.sh` after building and before the atomic push.
"""

from __future__ import annotations

from pathlib import Path

RELEASE = Path(__file__).resolve().parents[1] / "release.sh"


def _code_lines() -> list[str]:
    return [line.strip() for line in RELEASE.read_text().splitlines() if not line.strip().startswith("#")]


def _first(lines: list[str], needle: str) -> int:
    for i, line in enumerate(lines):
        if needle in line:
            return i
    msg = f"release.sh has no line running {needle!r}"
    raise AssertionError(msg)


def test_the_release_installs_after_building_and_before_pushing() -> None:
    lines = _code_lines()
    build = _first(lines, "./build.sh")
    install = _first(lines, "./install.sh")
    push = _first(lines, "git push --atomic")
    if not build < install < push:
        msg = f"release.sh must build, then install, then push (lines {build}, {install}, {push} of its code)"
        raise AssertionError(msg)
