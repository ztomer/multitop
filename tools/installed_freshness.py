#!/usr/bin/env python3
"""Fail when the INSTALLED `multitop` is not byte-identical to this build.

    python3 tools/installed_freshness.py [--installed PATH]

# Why this exists

Every end-to-end suite in this repo drives a binary. None of them drove
*this* one. On 2026-10-03 the installed `/opt/homebrew/bin/multitop` was
5,412,624 bytes and the working tree's own release build of the same commit was
5,477,232 -- same source, +64,608 bytes, +1.19% -- because `rustup` replaced
the stable toolchain on 2026-10-01, after that binary was built. So the
artifact under test was one the current compiler would not produce, and the
symptom would have been a bug report nobody could reproduce.

**Nothing else in this repo can see that.** The version string matches, because
the source matches. The tests pass, because they test the source. Only the
BYTES differ, and nothing compared them -- which is why this check is here
rather than the fact it reports.

Byte comparison rather than mtime or version: a toolchain change moves the
bytes without moving the version, and an mtime comparison cannot see content at
all.

# What it does, in order

1. `cargo build --release -p multitop`, so CARGO decides whether the build
   tree is current -- not an mtime heuristic that a `touch` defeats. A no-op
   when nothing changed, which is the case on a clean pre-push.

   There is no `--no-build` and there was one for an afternoon, and it is worth
   naming why it went. Comparing whatever is already in the build tree against
   whatever is installed reported GREEN here, byte-identical, sha256
   `48b4b556` -- while `target/release/multitop` was itself the 2026-09-30 build
   that this gate exists to catch, 64,608 bytes short of what the current
   toolchain produces. Two stale artifacts compare equal to each other. A mode
   that can report a success it did not earn is not a mode, it is a hole with a
   flag on it, so it is gone: the build is not optional.
2. Resolves the built binary through `cargo metadata`, because a shared
   `CARGO_TARGET_DIR` relocates it (same reason `install.sh` asks rather than
   assuming `target/`).
3. Resolves the installed binary: `$MULTITOP_INSTALLED`, else the first
   `multitop` on `PATH`, else `$(brew --prefix)/bin/multitop`.
4. Compares sha256 and, on a difference, prints BOTH sizes, BOTH digests, both
   mtimes and the one command that fixes it.

# Skipping, and why a skip has to be written down

A machine deliberately running a pinned older build is a legitimate state --
during a bisect, or while reproducing a report against the released binary --
and a gate that cannot be quiet about its state is a gate people switch off.
So `tools/installed_freshness_allow.json` carries one optional key:

    {"skip": "<why this machine runs a build that is not HEAD's>"}

An empty `{}` (what ships) means no skip. Two properties make it a ratchet
rather than a rug:

* An empty or whitespace-only reason is a hard FAILURE. The escape hatch
  cannot be a bare `{"skip": true}`.
* A skip that is no longer needed FAILS. If the binaries match and an
  allowance exists, the allowance is stale and the gate says to delete it --
  so the file cannot accumulate reasons for states that have passed.

Nothing is ever a silent pass: a missing install, a missing build and an
allowance each print what they are before exiting 0.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
from datetime import datetime, timezone
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
ALLOW = REPO / "tools" / "installed_freshness_allow.json"
BINARY = "multitop"


def info(msg: str) -> None:
    print(f"· {msg}")


def ok(msg: str) -> None:
    print(f"✓ {msg}")


def err(msg: str) -> None:
    print(f"✗ {msg}", file=sys.stderr)


def digest(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as fh:
        for chunk in iter(lambda: fh.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


_SIGN_ID = re.compile(r"codesign[^\n]*--identifier\s+(\S+)")


def install_identifier() -> str | None:
    """The signing identity `install.sh` uses, read out of `install.sh`.

    Read rather than repeated here: a second copy of the identifier is a copy
    that drifts, and a gate comparing against the wrong identity is a gate that
    goes red for a reason nobody can see. The regex skips the prose in the
    comment above the command by requiring `codesign` on the same line.
    """
    try:
        text = (REPO / "install.sh").read_text(encoding="utf-8")
    except OSError:
        return None
    found = _SIGN_ID.findall(text)
    return found[0] if len(found) == 1 else None


def as_installed(built: Path, scratch: Path) -> Path:
    """The bytes `install.sh` puts on disk: a copy of the build, signed as it signs.

    Measured, not assumed (2026-10-05), and the first version of this gate got
    it wrong. Comparing the RAW build against the raw install is not the
    invariant, because they are never the same file:

      * `cp` is byte-exact -- verified, sha256 unchanged across the copy;
      * `codesign -s - --identifier com.ztomer.multitop` on that copy
        reproduces the installed file's sha256 EXACTLY, and is idempotent, so
        running `install.sh` twice changes nothing;
      * but a freshly LINKED build carries the LINKER's ad-hoc signature --
        `Identifier=multitop-ec704f75be6b2efb`, `flags=adhoc,linker-signed`,
        CodeDirectory 42840 bytes against the installed 10860. Apple's linker
        signs arm64 binaries itself, so any relink replaces the house identity.

    So a gate comparing raw build bytes is red on arrival forever, which is the
    "gate people `--no-verify` past" outcome, arrived at honestly. The question
    worth asking is whether the installed binary is what `install.sh` would
    produce from this build, and that is what this returns.
    """
    staged = scratch / "as-installed"
    shutil.copy2(built, staged)
    os.chmod(staged, 0o755)
    if shutil.which("codesign"):
        identifier = install_identifier()
        if identifier is None:
            raise SystemExit(
                "installed_freshness: could not read exactly one "
                "`codesign --identifier` out of install.sh. The comparison has to "
                "sign the build the way the installer does, and guessing an "
                "identity would be a gate comparing against the wrong thing."
            )
        subprocess.run(
            ["codesign", "-s", "-", "--identifier", identifier, str(staged)],
            check=True, capture_output=True, text=True,
        )
    return staged


def when(path: Path) -> str:
    stamp = datetime.fromtimestamp(path.stat().st_mtime, tz=timezone.utc)
    return stamp.strftime("%Y-%m-%d %H:%M:%SZ")


def built_binary() -> Path:
    """The release binary, where cargo actually put it."""
    proc = subprocess.run(
        ["cargo", "metadata", "--format-version", "1"],
        capture_output=True,
        text=True,
        cwd=REPO,
        check=False,
    )
    target = REPO / "target"
    if proc.returncode == 0:
        try:
            named = json.loads(proc.stdout).get("target_directory")
            if named:
                target = Path(named)
        except json.JSONDecodeError:
            pass
    return target / "release" / BINARY


def installed_binary() -> Path | None:
    """Where the machine's `multitop` is, in the order install.sh implies."""
    override = os.environ.get("MULTITOP_INSTALLED")
    if override:
        return Path(override)
    from shutil import which

    on_path = which(BINARY)
    if on_path:
        return Path(on_path)
    brew = subprocess.run(
        ["brew", "--prefix"], capture_output=True, text=True, check=False
    )
    if brew.returncode == 0 and brew.stdout.strip():
        candidate = Path(brew.stdout.strip()) / "bin" / BINARY
        if candidate.is_file():
            return candidate
    return None


def allowance() -> str | None:
    """The recorded reason for skipping, or None. An empty reason is a failure."""
    if not ALLOW.is_file():
        return None
    try:
        data = json.loads(ALLOW.read_text(encoding="utf-8"))
    except json.JSONDecodeError as exc:
        err(f"installed_freshness: {ALLOW.name} is not valid JSON: {exc}")
        raise SystemExit(1) from exc
    reason = data.get("skip")
    if reason is None:
        return None
    if not isinstance(reason, str) or not reason.strip():
        err(
            f"installed_freshness: {ALLOW.name} has a skip with no reason. The escape "
            "hatch exists for a machine deliberately running a pinned older build, "
            "and a reasonless skip is indistinguishable from the gate being off."
        )
        raise SystemExit(1)
    return reason.strip()


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--installed", default="", help="path to the installed binary")
    args = ap.parse_args()

    built = built_binary()

    # CARGO decides freshness, not an mtime: a touch, a restored cache or a
    # half-finished build all defeat an mtime comparison, and this gate is about
    # the bytes, so the bytes had better come from a build cargo believes in.
    # There is deliberately no way to skip this -- see the module docstring.
    proc = subprocess.run(
        ["cargo", "build", "--release", "-p", BINARY],
        cwd=REPO,
        capture_output=True,
        text=True,
        check=False,
    )
    if proc.returncode != 0:
        err("installed_freshness: the release build failed")
        print(proc.stderr[-4000:], file=sys.stderr)
        return 1

    if not built.is_file():
        info(
            f"installed_freshness: no build at {built} -- NOT CHECKED. Run ./build.sh "
            "to produce one; this gate compares two binaries and one of them is absent."
        )
        return 0

    installed = Path(args.installed) if args.installed else installed_binary()
    if installed is None or not installed.is_file():
        where = args.installed or os.environ.get("MULTITOP_INSTALLED") or "$PATH, brew --prefix"
        info(
            f"installed_freshness: nothing installed (looked at {where}) -- NOT CHECKED. "
            "A machine that does not install this build has nothing to be stale about."
        )
        return 0

    reason = allowance()
    # The staged copy is deleted with the scratch directory, so its size is
    # read here rather than in the message below, which runs after it is gone.
    with tempfile.TemporaryDirectory() as scratch:
        expected = as_installed(built, Path(scratch))
        built_sum, installed_sum = digest(expected), digest(installed)
        expected_size = expected.stat().st_size
    same = built_sum == installed_sum

    if same:
        if reason:
            err(
                f"installed_freshness: {ALLOW.name} allows a stale install, and the "
                "installed binary now MATCHES the build. The allowance is stale: "
                f"delete the `skip` key from {ALLOW.name}."
            )
            return 1
        ok(
            f"installed {installed} is exactly what ./install.sh would put there "
            f"from {built} (sha256 {built_sum[:12]}, {installed.stat().st_size} bytes)"
        )
        return 0

    if reason:
        info(
            f"installed_freshness: SKIPPED with a stated reason -- {reason}\n"
            f"  installed {installed}  {installed.stat().st_size} bytes  "
            f"sha256 {installed_sum[:12]}  built {when(installed)}\n"
            f"  this build as installed  {expected_size} bytes  "
            f"sha256 {built_sum[:12]}  (built {when(built)}, signed as install.sh signs)\n"
            f"  recorded in {ALLOW.name}; delete its `skip` key when this machine is "
            "back on HEAD's build."
        )
        return 0

    hint = ""
    if not os.environ.get("MULTITOP_INSTALLED") and args.installed == "":
        hint = (
            "\n  A machine deliberately running a pinned older build records why in\n"
            f"  {ALLOW.relative_to(REPO)} instead."
        )
    err(
        "installed_freshness: the installed binary is NOT this build.\n"
        f"  installed {installed}  {installed.stat().st_size} bytes  "
        f"sha256 {installed_sum[:12]}  built {when(installed)}\n"
        f"  this build as installed  {expected_size} bytes  "
        f"sha256 {built_sum[:12]}  (built {when(built)}, signed as install.sh signs)\n"
        "\n"
        "  Same version string, different bytes: the usual cause is a toolchain\n"
        "  change since the install, which moves the bytes without moving the\n"
        "  version. Every end-to-end suite drives one of these two binaries.\n"
        f"\n  Fix: ./install.sh{hint}"
    )
    return 1


if __name__ == "__main__":
    sys.exit(main())