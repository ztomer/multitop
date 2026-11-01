"""A crate kept outside the workspace must be its own workspace root.

`fuzz/` is excluded from the root workspace, and that is enough only while the
checkout is not itself inside another cargo workspace. cargo finds a package's
workspace by walking UP from its manifest, so in a git worktree under
`.claude/worktrees/<name>/` -- inside the main checkout -- the walk from
`fuzz/Cargo.toml` passed the worktree's own root and reached the MAIN
checkout's `Cargo.toml`, whose `exclude = ["fuzz"]` names a different
directory. Every cargo call on the fuzz crate then died with "current package
believes it's in a workspace when it's not", and with it the commit gate's
fuzz check, in every worktree (2026-11-01, roadmap M1's commit).

The class: every manifest the root workspace excludes declares its own
`[workspace]`, so its root is itself wherever the checkout sits. Proved the way
it failed: the repository is reached through a parent directory that IS a cargo
workspace, and cargo is asked where the excluded crate's workspace root is.
"""

from __future__ import annotations

import json
import pathlib
import subprocess
import tomllib

import pytest

ROOT = pathlib.Path(__file__).resolve().parent.parent


def _excluded() -> list[str]:
    root = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))
    return list(root.get("workspace", {}).get("exclude", []))


def test_the_check_has_a_subject() -> None:
    # A scan over zero excluded crates reports compliance it did not earn.
    if not _excluded():
        msg = "the root workspace excludes nothing; this test checks nothing"
        raise AssertionError(msg)


@pytest.mark.parametrize("crate", _excluded())
def test_an_excluded_crate_is_its_own_root_inside_a_parent_workspace(crate: str, tmp_path: pathlib.Path) -> None:
    (tmp_path / "Cargo.toml").write_text("[workspace]\nmembers = []\n", encoding="utf-8")
    # A symlink, not a copy: cargo walks the path it was given, so this places
    # the real tree inside a parent workspace exactly as a nested worktree is.
    (tmp_path / "checkout").symlink_to(ROOT, target_is_directory=True)
    manifest = tmp_path / "checkout" / crate / "Cargo.toml"
    command = ["cargo", "metadata", "--manifest-path", str(manifest), "--no-deps", "--offline", "--format-version", "1"]
    got = subprocess.run(  # noqa: S603
        command,
        stdin=subprocess.DEVNULL,
        capture_output=True,
        text=True,
        check=False,
        timeout=120,
    )
    if got.returncode != 0:
        msg = f"cargo could not load {crate}/Cargo.toml inside a parent workspace:\n{got.stderr}"
        raise AssertionError(msg)
    workspace_root = pathlib.Path(json.loads(got.stdout)["workspace_root"])
    if workspace_root != manifest.parent:
        msg = f"{crate} resolved to the workspace at {workspace_root}"
        raise AssertionError(msg)
