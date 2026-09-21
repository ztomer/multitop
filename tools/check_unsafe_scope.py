#!/usr/bin/env python3
"""Fail if `unsafe` appears in a Rust file that is not on the FFI allowlist.

`unsafe_code` cannot be a workspace lint: the agent reads the kernel through
libc and Mach, and the vault pins its key material with `mlock`, so both
crates need `unsafe` in a handful of modules. The old arrangement was a
workspace-wide deny plus a per-module `#[expect(unsafe_code)]` carving each
of those modules out. The house policy of 2026-09-20 refuses `#[expect]` and
`#[allow]` in Rust, so the carve-out is held HERE instead: this file is the
complete list of modules that may contain `unsafe`, and a new module that
grows an `unsafe` block fails the gate until it is added to the list -- in
a diff a reviewer sees, next to the reason.

`multitop` itself has no FFI and keeps `#![deny(unsafe_code)]` at its crate
roots; this checker is the equivalent for the two crates that cannot.

The list is a ratchet in both directions: a listed file that no longer
contains `unsafe` fails too, so the list never rots into a set of blanket
permissions nobody remembers granting.

Run with --self-test to check the checker.
"""

from __future__ import annotations

import re
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from _scope import scope_is_empty  # noqa: E402

CRATES = Path("crates")

# Every file that may contain `unsafe`, with the reason it needs to.
ALLOWED: dict[str, str] = {
    "crates/agent/src/exec/lock.rs": "flock(2) on the exec lock file",
    "crates/agent/src/exec/pty.rs": "openpty/fork/ioctl for the command PTY",
    "crates/agent/src/exec/pump.rs": "raw read(2)/close(2) on the PTY master",
    "crates/agent/src/fetch.rs": "sysctl/uname for the fetch card",
    "crates/agent/src/monitor.rs": "Mach host statistics on macOS",
    "crates/agent/src/proc.rs": "proc_listpids/proc_pidinfo on macOS",
    "crates/agent/src/proc_disk.rs": "proc_pid_rusage for per-process disk I/O",
    "crates/agent/src/proc_sys.rs": "sysctl process table on macOS",
    "crates/agent/src/sys.rs": "sysctl/Mach sampling on macOS",
    "crates/agent/src/sys_temps.rs": "IOKit HID temperature sensors on macOS",
    "crates/vault/src/mlock.rs": "mlock/munlock to pin key material in RAM",
}

# The keyword itself. A comment or string that merely mentions it is stripped
# below; what is left is code, where `unsafe` is only ever the keyword.
UNSAFE = re.compile(r"\bunsafe\b")
STRING = re.compile(r'"(?:\\.|[^"\\])*"')


def has_unsafe(text: str) -> bool:
    for line in text.splitlines():
        code = STRING.sub('""', line.split("//", 1)[0])
        if UNSAFE.search(code):
            return True
    return False


def audit(root: Path, allowed: dict[str, str]) -> tuple[list[Path], list[Path], int]:
    """(unlisted files with unsafe, listed files without unsafe, files scanned)."""
    unlisted: list[Path] = []
    seen: set[str] = set()
    scanned = 0
    for path in sorted(root.rglob("*.rs")):
        scanned += 1
        key = path.as_posix()
        found = has_unsafe(path.read_text(encoding="utf-8"))
        if found:
            seen.add(key)
            if key not in allowed:
                unlisted.append(path)
    stale = [Path(k) for k in allowed if k not in seen]
    return unlisted, stale, scanned


def self_test() -> int:
    with tempfile.TemporaryDirectory() as tmp:
        root = Path(tmp)
        src = root / "c" / "src"
        src.mkdir(parents=True)
        ffi = src / "ffi.rs"
        ffi.write_text("pub fn f() { unsafe { libc::getpid() }; }", encoding="utf-8")
        key = ffi.as_posix()

        unlisted, stale, _ = audit(root, {key: "test"})
        if unlisted or stale:
            print("self-test FAILED: a listed unsafe file was reported")
            return 1

        unlisted, _, _ = audit(root, {})
        if not unlisted:
            print("self-test FAILED: an unlisted unsafe block was not detected")
            return 1

        ffi.write_text("pub fn f() {}", encoding="utf-8")
        _, stale, _ = audit(root, {key: "test"})
        if not stale:
            print("self-test FAILED: a stale allowlist entry was not detected")
            return 1

        ffi.write_text('// unsafe here once\nlet s = "unsafe";', encoding="utf-8")
        unlisted, _, _ = audit(root, {})
        if unlisted:
            print("self-test FAILED: a comment or string was reported as code")
            return 1

        ffi.write_text("fn unsafe_ish() {}", encoding="utf-8")
        unlisted, _, _ = audit(root, {})
        if unlisted:
            print("self-test FAILED: an identifier containing the word was reported")
            return 1
    print("check_unsafe_scope self-test: ok")
    return 0


def main() -> int:
    if "--self-test" in sys.argv:
        return self_test()
    if not CRATES.is_dir():
        print(f"unsafe-scope: {CRATES} not found -- run from the repository root")
        return 1
    unlisted, stale, scanned = audit(CRATES, ALLOWED)
    if scope_is_empty("unsafe-scope", scanned, "Rust files under crates/"):
        return 1
    if not unlisted and not stale:
        print(f"unsafe-scope: clean ({scanned} files checked, {len(ALLOWED)} may contain unsafe)")
        return 0
    if unlisted:
        print("unsafe-scope: `unsafe` in a file that is not on the FFI allowlist\n")
        for path in unlisted:
            print(f"  {path}")
        print(
            "\nIf the unsafe is genuinely needed, add the file to ALLOWED in\n"
            "tools/check_unsafe_scope.py with the reason. Never annotate the\n"
            "module with #[allow]/#[expect]; the house gate refuses both."
        )
    if stale:
        print("unsafe-scope: allowlisted files that no longer contain `unsafe`\n")
        for path in stale:
            print(f"  {path}")
        print("\nRemove them from ALLOWED so the list stays exactly the FFI surface.")
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
