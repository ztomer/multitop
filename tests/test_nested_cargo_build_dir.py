"""A build script that runs cargo must give that cargo its own build-dir.

The host config keys `build-dir` by workspace path (~/.cargo/build/<hash>), so a
cargo spawned from a build script of the SAME workspace resolves the build-dir
lock the outer cargo holds while the script runs. Both then wait at 0% CPU with
no output and no end: the pre-push's release build in a fresh worktree hung
past its 900 s ceiling twice on 2026-10-18 this way, in
crates/multitop/build.rs's agent auto-build. Only a fresh tree shows it -- any
tree where an earlier step left an agent binary skips the nested build.

The class, not the instance: every `Command::new("cargo")` in any build.rs of
this workspace must set CARGO_BUILD_BUILD_DIR in the same command chain.
"""

from __future__ import annotations

import pathlib
import re

ROOT = pathlib.Path(__file__).resolve().parent.parent
SPAWN = re.compile(r'Command::new\("cargo"\)')


def _build_scripts() -> list[pathlib.Path]:
    return sorted(p for p in (ROOT / "crates").glob("*/build.rs"))


def _unguarded(text: str) -> list[int]:
    """Line numbers of cargo spawns whose command chain (to its `.status()` /
    `.output()` / `.spawn()`) never sets CARGO_BUILD_BUILD_DIR."""
    bad = []
    for m in SPAWN.finditer(text):
        end = re.compile(r"\.(status|output|spawn)\(\)").search(text, m.end())
        chain = text[m.end() : end.start() if end else len(text)]
        if "CARGO_BUILD_BUILD_DIR" not in chain:
            bad.append(text.count("\n", 0, m.start()) + 1)
    return bad


def test_the_scan_has_a_subject() -> None:
    # A scan over zero build scripts, or zero spawns, reports compliance it did
    # not earn.
    scripts = _build_scripts()
    assert scripts, "no build.rs found under crates/"
    assert any(SPAWN.search(p.read_text()) for p in scripts), "no cargo spawn to check"


def test_every_nested_cargo_has_its_own_build_dir() -> None:
    offenders = {
        str(p.relative_to(ROOT)): lines
        for p in _build_scripts()
        if (lines := _unguarded(p.read_text()))
    }
    assert not offenders, f"nested cargo sharing the outer build-dir lock: {offenders}"


def test_the_check_sees_an_unguarded_spawn() -> None:
    bare = 'let s = std::process::Command::new("cargo").args(["build"]).status();'
    guarded = (
        'let s = std::process::Command::new("cargo").args(["build"])'
        '.env("CARGO_BUILD_BUILD_DIR", d).status();'
    )
    assert _unguarded(bare) == [1]
    assert _unguarded(guarded) == []
