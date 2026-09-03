#!/usr/bin/env python3
"""Ensure the embedded agent matches the workspace version.

The workspace version (Cargo.toml `version`) is baked into the agent binary via
`AGENT_VERSION = env!("CARGO_PKG_VERSION")`. If the agent on disk was built
from an older checkout, its bytes still contain the old version string, so the
`multitop` binary that embeds it will Hello `0.44.0` while itself is `0.44.1`
and the `replace_agent` loop uploads the same stale bytes forever.

This is the same check `crates/multitop/build.rs` does at compile time for
release builds, but that gate is silent for debug builds and never runs when
someone does `cargo test` without rebuilding the agent. A standalone gate that
runs in the hook, in CI and in `local-ci.py` catches the stale-agent case
wherever it happens.

Usage:
    python3 tools/check_agent_version.py [--self-test]

Exit 1 when a stale agent would be embedded.
"""

from __future__ import annotations

import re
import sys
import tempfile
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
CARGO_TOML = REPO / "Cargo.toml"

# Same discovery as build.rs, minus the OUT_DIR climbing (which is build-specific).
# Keep the two in sync: if build.rs gains a new root, add it here.
CANDIDATE_ROOTS = [
    REPO / "target",
    REPO / "target" / "docker",
    REPO / "target" / "agent-build",
    Path.home() / ".cache" / "cargo-target",
]

# Env var takes precedence, just like build.rs
ENV_VARS = {
    "x86_64-unknown-linux-musl": "MULTITOP_AGENT_X86_64",
    "aarch64-unknown-linux-musl": "MULTITOP_AGENT_AARCH64",
}


def workspace_version() -> str:
    text = CARGO_TOML.read_text(encoding="utf-8")
    m = re.search(r'(?m)^version = "([^"]+)"', text)
    if not m:
        print("check_agent_version: could not find workspace version in Cargo.toml", file=sys.stderr)
        sys.exit(2)
    return m.group(1)


def find_agent(triple: str) -> Path | None:
    env_var = ENV_VARS[triple]
    explicit = __import__("os").environ.get(env_var, "")
    if explicit:
        p = Path(explicit)
        if p.is_file():
            return p
    # Also check CARGO_TARGET_DIR
    import os

    target_dir = os.environ.get("CARGO_TARGET_DIR")
    roots = list(CANDIDATE_ROOTS)
    if target_dir:
        roots.insert(0, Path(target_dir))
    for root in roots:
        for profile in ("release", "debug"):
            p = root / triple / profile / "multitop-agent"
            if p.is_file():
                return p
    return None


def check_one(triple: str, ws_version: str) -> str | None:
    path = find_agent(triple)
    if path is None:
        # No agent is okay for local-only debug and for CI's `gates` job
        # which doesn't cross-build the musl agents. The `build.rs` gate
        # already warns for release builds without an agent; this gate only
        # fails when a stale agent *would* be embedded.
        return None
    data = path.read_bytes()
    if ws_version.encode() not in data:
        return (
            f"{triple}: workspace {ws_version} not inside {path} "
            f"(stale agent, likely {ws_version} bump without rebuild) — rebuild with ./build.sh"
        )
    return None


def self_test() -> int:
    # Prove it can find a mismatch before trusting it to report clean.
    with tempfile.TemporaryDirectory() as tmp:
        # Fake agent containing 0.44.0, workspace says 0.44.1
        agent = Path(tmp) / "multitop-agent"
        agent.write_bytes(b"multitop-agent 0.44.0")
        # Monkey-patch find_agent to return our fake
        orig = find_agent

        def fake_find(triple: str) -> Path | None:
            return agent if triple == "x86_64-unknown-linux-musl" else None

        globals()["find_agent"] = fake_find
        # Should flag the stale x86_64
        err = check_one("x86_64-unknown-linux-musl", "0.44.1")
        if err is None or "stale" not in err:
            print("self-test: stale agent was NOT flagged", file=sys.stderr)
            globals()["find_agent"] = orig
            return 1
        # Should pass when versions match
        agent.write_bytes(b"multitop-agent 0.44.1")
        err2 = check_one("x86_64-unknown-linux-musl", "0.44.1")
        if err2 is not None:
            print(f"self-test: matching agent was flagged: {err2}", file=sys.stderr)
            globals()["find_agent"] = orig
            return 1
        globals()["find_agent"] = orig
    print("check_agent_version self-test: passed")
    return 0


def main() -> int:
    if "--self-test" in sys.argv:
        return self_test()
    ws = workspace_version()
    problems: list[str] = []
    for triple in ("x86_64-unknown-linux-musl", "aarch64-unknown-linux-musl"):
        if (err := check_one(triple, ws)) is not None:
            problems.append(err)
    if not problems:
        print(f"agent-version: clean (workspace {ws} inside both agents)")
        return 0
    print("agent-version: stale agent would be embedded\n")
    for p in problems:
        print(f"  {p}")
    print("\n  Fix: ./build.sh  (rebuilds both musl agents and re-embeds them)")
    print("  Or: cargo build -p multitop --release after ./build.sh")
    return 1


if __name__ == "__main__":
    sys.exit(main())
