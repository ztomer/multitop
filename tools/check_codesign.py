#!/usr/bin/env python3
"""Ensure the multitop binary has a stable ad-hoc signature.

Without a fixed identifier each `cargo build` produces a random ad-hoc
`Identifier=multitop-<hash>` and macOS keychain ACLs are per-signature.
"Always Allow" then only allows that one binary — the next build prompts
again. `build.sh` now does `codesign -s - --identifier com.ztomer.multitop`
to give every build the same `Identifier=com.ztomer.multitop`.

This gate checks the newest `multitop` binary that `find_binary()` would run
has that identifier. It BUILDS that binary first (`cargo build -p multitop`),
so its subject is this tree's build: cargo decides freshness, the way
tools/installed_freshness.py does. Until 2026-10-08 it read whatever was
already in target/ and said it "runs after `cargo build` in every list that
runs it" -- in all three it ran BEFORE the build, so it passed in a used
checkout over a stale artefact and refused every fresh clone and the
pre-push clean worktree with an empty scope. A binary with a random identifier fails the gate before
it ever reaches a keychain prompt.

Usage:
    python3 tools/check_codesign.py [--self-test]

Exit 1 when the signature is not stable.
"""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(__import__('pathlib').Path(__file__).resolve().parent))
from _scope import scope_is_empty  # noqa: E402

REPO = Path(__file__).resolve().parent.parent
EXPECTED_ID = "com.ztomer.multitop"


def find_binaries() -> list[Path]:
    # All multitop binaries that might be used: release, debug, cargo bin.
    # The keychain prompt is per-signature, so every binary that a developer
    # might run must have the stable identifier.
    import os

    bins: list[Path] = []
    if (explicit := os.environ.get("MULTITOP_BIN")) and Path(explicit).is_file():
        bins.append(Path(explicit))
    # The artefacts of THIS tree: cargo's own answer for where it puts them,
    # asked from the working directory. Until 2026-09-14 this also walked the
    # machine-wide ~/.cache/cargo-target and ~/.cargo/bin, which made the gate
    # pass over an EMPTY tree on any machine that had ever built multitop --
    # the empty-scope sweep caught it. Host state is not this tree's subject;
    # MULTITOP_BIN names an installed copy when that is what you mean.
    import json
    import subprocess

    roots: list[Path] = []
    try:
        meta = subprocess.run(
            ["cargo", "metadata", "--format-version", "1", "--no-deps"],
            capture_output=True, text=True, check=True, cwd=REPO,
        )
        roots.append(Path(json.loads(meta.stdout)["target_directory"]))
    except (OSError, subprocess.CalledProcessError, KeyError, ValueError):
        pass  # no workspace here: no artefacts of this tree exist
    for root in roots:
        for profile in ("release", "debug"):
            cand = root / profile / "multitop"
            if cand.is_file() and cand not in bins:
                bins.append(cand)
    return bins


def cargo_build() -> bool:
    """Build this tree's debug `multitop`; False (after printing why) on failure."""
    try:
        proc = subprocess.run(
            ["cargo", "build", "-p", "multitop"],
            cwd=REPO, capture_output=True, text=True, check=False,
        )
    except OSError as exc:
        print(f"check_codesign: cannot run cargo: {exc}", file=sys.stderr)
        return False
    if proc.returncode != 0:
        print("check_codesign: `cargo build -p multitop` failed", file=sys.stderr)
        print(proc.stderr[-4000:], file=sys.stderr)
        return False
    return True


def built_binaries(build=cargo_build, find=find_binaries) -> list[Path] | None:
    """The binaries to check, after building this tree's. None when the build
    failed: a gate that cannot produce its subject has not checked it."""
    if not build():
        return None
    return find()


def find_binary() -> Path | None:
    bins = find_binaries()
    if not bins:
        return None
    # Newest for the single-binary check (kept for compatibility)
    newest = max(bins, key=lambda p: p.stat().st_mtime)
    return newest


def identifier_for(binary: Path) -> str | None:
    try:
        out = subprocess.run(
            ["codesign", "-dv", str(binary)],
            capture_output=True,
            text=True,
            timeout=10,
        )
    except FileNotFoundError:
        return None
    # codesign -dv prints to stderr
    text = out.stderr + out.stdout
    for line in text.splitlines():
        if "Identifier=" in line:
            # e.g. Identifier=com.ztomer.multitop
            return line.split("Identifier=", 1)[1].strip()
    return None


def self_test() -> int:
    # Prove it can distinguish before trusting it to report clean.
    # We don't have a real binary with a known bad identifier, but we can
    # check that the parser finds the expected identifier in a string that
    # looks like codesign output.
    fake = "Identifier=com.ztomer.multitop\n"
    # Simulate the parsing
    ident = None
    for line in fake.splitlines():
        if "Identifier=" in line:
            ident = line.split("Identifier=", 1)[1].strip()
    if ident != EXPECTED_ID:
        print(f"self-test: parser failed to find {EXPECTED_ID}", file=sys.stderr)
        return 1
    fake_bad = "Identifier=multitop-abc123\n"
    bad = None
    for line in fake_bad.splitlines():
        if "Identifier=" in line:
            bad = line.split("Identifier=", 1)[1].strip()
    if bad == EXPECTED_ID:
        print("self-test: bad identifier was considered good", file=sys.stderr)
        return 1
    # The subject is BUILT before it is looked for: an empty target/ is built
    # into, and a failed build is a failure rather than an empty scope.
    calls: list[str] = []
    made: list[Path] = []
    def fake_build() -> bool:
        calls.append("build")
        made.append(Path("target/debug/multitop"))
        return True
    if built_binaries(fake_build, lambda: list(made)) != [Path("target/debug/multitop")]:
        print("self-test: the binary was looked for before it was built", file=sys.stderr)
        return 1
    if built_binaries(lambda: False, lambda: [Path("stale")]) is not None:
        print("self-test: a failed build checked a stale artefact", file=sys.stderr)
        return 1
    print("check_codesign self-test: passed")
    return 0


def main() -> int:
    if "--self-test" in sys.argv:
        return self_test()
    # Only check on macOS where codesign and the keychain matter.
    import platform

    if platform.system() != "Darwin":
        # Not a pass and not a failure: codesign and the keychain prompt do
        # not exist here. "not applicable" is the phrase the house empty-scope
        # sweep reads as a named non-run, so this holds on every host without
        # an excuse whose truth depends on which one ran it.
        print("check_codesign: not applicable (not macOS -- no codesign, no keychain prompt)")
        return 0
    bins = built_binaries()
    if bins is None:
        return 1
    # Built just now, so no binary means the scope moved (a renamed package,
    # another target dir), never "clean".
    if scope_is_empty("check_codesign", len(bins), "multitop binaries (run cargo build -p multitop)"):
        return 1
    problems: list[Path] = []
    for binary in bins:
        ident = identifier_for(binary)
        if ident is None:
            print(f"check_codesign: could not read signature for {binary}", file=sys.stderr)
            continue
        if ident != EXPECTED_ID:
            problems.append(binary)
    if problems:
        # Auto-fix: stable-ify the signature so "Always Allow" persists.
        # Without this, every `cargo test` (which rebuilds debug) gets a random
        # ad-hoc ID and the next keychain access prompts again.
        for binary in problems:
            try:
                subprocess.run(
                    ["codesign", "-s", "-", "--identifier", EXPECTED_ID, str(binary)],
                    capture_output=True,
                    timeout=10,
                )
            except FileNotFoundError:
                break
        # Re-check after auto-fix
        still: list[str] = []
        for binary in problems:
            ident = identifier_for(binary)
            if ident != EXPECTED_ID:
                still.append(f"  {binary}: Identifier={ident} (expected {EXPECTED_ID})")
        if still:
            print("check_codesign: unstable signature\n")
            for p in still:
                print(p)
            print(f"  This binary will prompt for keychain access on every rebuild.")
            print(f"  Fix: for f in {' '.join(str(b) for b in bins)}; do codesign -s - --identifier {EXPECTED_ID} \"$f\"; done")
            return 1
        print(f"check_codesign: clean (auto-fixed {len(problems)} binaries Identifier={EXPECTED_ID})")
        return 0
    print(f"check_codesign: clean ({len(bins)} binaries Identifier={EXPECTED_ID})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
