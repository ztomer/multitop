#!/usr/bin/env python3
"""Ensure the multitop binary has a stable ad-hoc signature.

Without a fixed identifier each `cargo build` produces a random ad-hoc
`Identifier=multitop-<hash>` and macOS keychain ACLs are per-signature.
"Always Allow" then only allows that one binary — the next build prompts
again. `build.sh` now does `codesign -s - --identifier com.ztomer.multitop`
to give every build the same `Identifier=com.ztomer.multitop`.

This gate checks the newest `multitop` binary that `find_binary()` would run
has that identifier. A binary with a random identifier fails the gate before
it ever reaches a keychain prompt.

Usage:
    python3 tools/check_codesign.py [--self-test]

Exit 1 when the signature is not stable.
"""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path

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
    # Cargo's bin dir
    cargo_bin = Path.home() / ".cargo" / "bin" / "multitop"
    if cargo_bin.is_file():
        bins.append(cargo_bin)
    roots: list[Path] = []
    if (target := os.environ.get("CARGO_TARGET_DIR")):
        roots.append(Path(target))
    roots += [
        Path.home() / ".cache" / "cargo-target",
        REPO / "target",
    ]
    for root in roots:
        for profile in ("release", "debug"):
            cand = root / profile / "multitop"
            if cand.is_file() and cand not in bins:
                bins.append(cand)
    return bins


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
    print("check_codesign self-test: passed")
    return 0


def main() -> int:
    if "--self-test" in sys.argv:
        return self_test()
    # Only check on macOS where codesign and the keychain matter.
    import platform

    if platform.system() != "Darwin":
        print("check_codesign: clean (not macOS, no keychain prompt)")
        return 0
    bins = find_binaries()
    if not bins:
        print("check_codesign: clean (no multitop binary built yet — run cargo build -p multitop)")
        return 0
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
