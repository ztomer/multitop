#!/usr/bin/env python3
"""Fail if a minimum Rust version is claimed that no build ever compiles with.

`rust-version = "1.85"` in Cargo.toml and "Rust 1.85+" in README.md were two
claims of one number, and nothing compiled with it: CI and the local gate both
float on `stable`. By 2026-10-10 the claim was already false -- the lock needed
1.89 (`aes` 0.9.3, `uuid` 1.27.0), so a user on 1.85 got a build error from a
crate they never chose, under a README that promised it would work. A number
nothing checks goes false on the first lockfile refresh after it is written,
and says nothing when it does. Removed 2026-10-10 (roadmap H2); this keeps it
from coming back as a bare number.

The rule: a minimum-version claim must name a toolchain something builds on --
the CI workflow installs it (`dtolnay/rust-toolchain@X`, or `toolchain: X`), or
`rust-toolchain.toml` pins `channel = "X"`. A claim is

    rust-version = "X"           in any tracked Cargo.toml ([package] or
                                 [workspace.package]); `rust-version.workspace
                                 = true` inherits one and is not a claim itself
    Rust X+ / Rust X or newer    in any tracked Markdown file
    MSRV X                       likewise

Restoring an MSRV is a fine outcome: add the job that builds on it, and this
passes. It is the number without the job that fails.

Run with --self-test to check the checker.
"""

from __future__ import annotations

import re
import subprocess
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from _scope import scope_is_empty  # noqa: E402

VERSION = r"(\d+\.\d+(?:\.\d+)?)"
MANIFEST_CLAIM = re.compile(r'^\s*rust-version\s*=\s*"' + VERSION + r'"', re.M)
PACKAGE = re.compile(r"^\[package\]", re.M)
PROSE_CLAIMS = (
    re.compile(r"\bRust\s+" + VERSION + r"\s*(?:\+|or (?:newer|later|above)\b)"),
    re.compile(r"\bMSRV\b[^0-9\n]{0,12}" + VERSION),
)
WORKFLOWS = Path(".github/workflows")
TOOLCHAIN_FILE = Path("rust-toolchain.toml")


def claims(root: Path, files: list[str]) -> list[tuple[str, int, str]]:
    """Every `(file, line, version)` claim in `files` (paths relative to `root`)."""
    out: list[tuple[str, int, str]] = []
    for rel in files:
        path = root / rel
        if not path.is_file():
            continue
        text = path.read_text(encoding="utf-8", errors="replace")
        patterns = (MANIFEST_CLAIM,) if rel.endswith("Cargo.toml") else PROSE_CLAIMS
        for pat in patterns:
            for m in pat.finditer(text):
                out.append((rel, text.count("\n", 0, m.start()) + 1, m.group(1)))
    return out


def exercised(root: Path) -> str:
    """Everything that says which toolchain a build installs: every workflow file
    (a disabled `ci.yml.disabled` included -- it is what gets re-enabled) and the
    toolchain file."""
    parts: list[str] = []
    wf = root / WORKFLOWS
    if wf.is_dir():
        parts += [p.read_text(encoding="utf-8") for p in sorted(wf.iterdir()) if p.is_file()]
    if (root / TOOLCHAIN_FILE).is_file():
        parts.append((root / TOOLCHAIN_FILE).read_text(encoding="utf-8"))
    return "\n".join(parts)


def builds_on(version: str, builds: str) -> bool:
    v = re.escape(version)
    return any(
        re.search(p, builds, re.M)
        for p in (
            rf"rust-toolchain@{v}(?![\d.])",
            rf"\btoolchain:\s*[\"']?{v}(?![\d.])",
            rf"^\s*channel\s*=\s*\"{v}\"",
        )
    )


def offenders(root: Path, files: list[str]) -> list[tuple[str, int, str]]:
    builds = exercised(root)
    return [c for c in claims(root, files) if not builds_on(c[2], builds)]


def self_test() -> int:
    cases = [
        # (files, workflow, toolchain file, offending-count, what it proves)
        (
            {"Cargo.toml": '[workspace.package]\nrust-version = "1.85"\n'},
            "",
            "",
            1,
            "a bare rust-version was not reported",
        ),
        ({"README.md": "Needs Rust 1.85+, ssh.\n"}, "", "", 1, "a README 'Rust X+' claim was not reported"),
        ({"docs/x.md": "MSRV: 1.80\n"}, "", "", 1, "an 'MSRV X' claim was not reported"),
        (
            {"a/Cargo.toml": "[package]\nrust-version.workspace = true\n"},
            "",
            "",
            0,
            "an inherited rust-version was reported as a claim",
        ),
        (
            {"Cargo.toml": 'rust-version = "1.85"\n', "README.md": "Rust 1.85+\n"},
            "      - uses: dtolnay/rust-toolchain@1.85\n",
            "",
            0,
            "a claim CI builds on was reported",
        ),
        (
            {"Cargo.toml": 'rust-version = "1.85"\n'},
            "        toolchain: '1.85'\n",
            "",
            0,
            "a `toolchain: X` job was not counted",
        ),
        (
            {"Cargo.toml": 'rust-version = "1.85"\n'},
            "",
            '[toolchain]\nchannel = "1.85"\n',
            0,
            "a pinned rust-toolchain.toml was not counted",
        ),
        (
            {"Cargo.toml": 'rust-version = "1.8"\n'},
            "uses: dtolnay/rust-toolchain@1.85\n",
            "",
            1,
            "1.85 in CI was read as building on 1.8",
        ),
        ({"README.md": "Rust (latest stable)\n"}, "", "", 0, "a claim-free line was reported"),
    ]
    for files, workflow, toolchain, want, why in cases:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            for rel, text in files.items():
                (root / rel).parent.mkdir(parents=True, exist_ok=True)
                (root / rel).write_text(text, encoding="utf-8")
            if workflow:
                (root / WORKFLOWS).mkdir(parents=True)
                (root / WORKFLOWS / "ci.yml").write_text(workflow, encoding="utf-8")
            if toolchain:
                (root / TOOLCHAIN_FILE).write_text(toolchain, encoding="utf-8")
            got = len(offenders(root, list(files)))
            if got != want:
                print(f"self-test FAILED: {why} (got {got} offender(s), want {want})")
                return 1
    print("check_msrv_claim self-test: ok")
    return 0


def tracked(pattern: str) -> list[str]:
    out = subprocess.run(
        ["git", "ls-files", "-z", "--", pattern],
        capture_output=True,
        text=True,
        check=True,
    ).stdout
    return [p for p in out.split("\0") if p]


def main() -> int:
    if "--self-test" in sys.argv:
        return self_test()
    manifests = tracked("*Cargo.toml")
    # The floor is CRATES, not manifest files: a workspace root alone (no
    # `[package]`) is a tree whose crates are gone, and "clean" over it would be
    # compliance over nothing (gates_of_heck's empty-scope sweep keeps exactly
    # that root as the skeleton's shape file).
    crates = [m for m in manifests if PACKAGE.search(Path(m).read_text(encoding="utf-8"))]
    if scope_is_empty("msrv-claim", len(crates), "crate manifests ([package])"):
        return 1
    files = manifests + tracked("*.md")
    hits = offenders(Path("."), files)
    if not hits:
        print(f"msrv-claim: clean ({len(files)} files checked)")
        return 0
    print("msrv-claim: a minimum Rust version is claimed that no build compiles with\n")
    for rel, line, version in hits:
        print(f"  {rel}:{line}  Rust {version}")
    print(
        "\nCI and the local gate float on stable, so this number is checked by\n"
        "nothing and goes false on the first lockfile refresh after it is written.\n"
        "Delete it, or add the job that builds on it (dtolnay/rust-toolchain@X in\n"
        ".github/workflows) -- then it is a claim with a witness."
    )
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
