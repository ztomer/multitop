"""`tools/ship_release.sh` is the release kit `goh ship` drives (MON.SHIPKIT, 2026-11-13).

monitor tagged releases (v0.69.7) and declared no `GOH_SHIP_RELEASE`, so `goh ship` refused it and
agents fell back to a bare `git push` with no tag and no install. `release.sh` cannot be the kit on
its own: it refuses unless the version bump and the CHANGELOG stanza are already committed, and
`goh ship` decides the number. The kit is the front half it lacks: write the version into the
workspace manifest and both lockfiles, cut the stanza from Unreleased, commit, then `./release.sh`
(which installs before it pushes, because the push gate compares the installed binary).

The cases run the REAL script in a throwaway repository whose `release.sh` is a recorder.
"""

from __future__ import annotations

import json
import os
import re
import shutil
import stat
import subprocess
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
KIT = REPO / "tools" / "ship_release.sh"
GATESRC = REPO / ".gatesrc"

_HOME_GOH = Path.home() / "Projects" / "gates_of_heck"
_ENV_GOH = Path(os.environ.get("GOH_DIR", ""))
GOH = _ENV_GOH if (_ENV_GOH / "gates" / "python.sh").is_file() else _HOME_GOH

MANIFEST = """[workspace]
members = ["crates/agent"]

[workspace.package]
version = "{v}"
edition = "2021"
"""

LOCK = """version = 4

[[package]]
name = "multitop"
version = "{v}"

[[package]]
name = "multitop-agent"
version = "{v}"

[[package]]
name = "multitop-testassert"
version = "{v}"

[[package]]
name = "multitop-vault"
version = "0.21.0"

[[package]]
name = "serde"
version = "{v}"
source = "registry+https://github.com/rust-lang/crates.io-index"
"""

FUZZ_LOCK = """version = 4

[[package]]
name = "multitop"
version = "{v}"

[[package]]
name = "multitop-agent"
version = "{v}"
"""

CHANGELOG = """# Changelog

## Unreleased

### Fixed
- The thing that was wrong is right now, so a release has something to say.

## v0.1.0 — the first _(2026-01-01)_

### Added
- It exists.
"""

RECORDER = """#!/usr/bin/env bash
echo "$@" > "$(dirname "$0")/tools/.release-args"
"""


def _run(cwd: Path, *argv: str, env: dict[str, str] | None = None, check: bool = True) -> subprocess.CompletedProcess:
    # A hook that runs this suite binds git to ITS repository (GIT_DIR, GIT_INDEX_FILE ...): drop it.
    full = {k: v for k, v in os.environ.items() if not k.startswith("GIT_")}
    full |= {"GIT_AUTHOR_NAME": "t", "GIT_AUTHOR_EMAIL": "t@t", "GIT_COMMITTER_NAME": "t"}
    full |= {"GIT_COMMITTER_EMAIL": "t@t", "GOH_DIR": str(GOH), **(env or {})}
    done = subprocess.run(  # noqa: S603
        list(argv), cwd=cwd, env=full, capture_output=True, text=True, stdin=subprocess.DEVNULL, check=False
    )
    if check:
        assert done.returncode == 0, f"{argv}: {done.stdout}\n{done.stderr}"  # noqa: S101
    return done


def _fixture(tmp: Path, *, tree_v: str = "0.1.0", changelog: str = CHANGELOG) -> Path:
    repo = tmp / "repo"
    (repo / "tools").mkdir(parents=True)
    (repo / "fuzz").mkdir()
    shutil.copy(KIT, repo / "tools" / "ship_release.sh")
    (repo / "release.sh").write_text(RECORDER)
    (repo / "release.sh").chmod(0o755)
    (repo / "Cargo.toml").write_text(MANIFEST.format(v=tree_v))
    (repo / "Cargo.lock").write_text(LOCK.format(v=tree_v))
    (repo / "fuzz" / "Cargo.lock").write_text(FUZZ_LOCK.format(v=tree_v))
    (repo / "CHANGELOG.md").write_text(changelog)
    _run(repo, "git", "init", "-q", "-b", "main")
    _run(repo, "git", "add", "-A")
    _run(repo, "git", "-c", "commit.gpgsign=false", "commit", "-q", "-m", "seed", "--no-verify")
    _run(repo, "git", "tag", "-a", "v0.1.0", "-m", "v0.1.0")
    return repo


def _kit(repo: Path, version: str, summary: str = "", check: bool = False) -> subprocess.CompletedProcess:
    env = {}
    if summary:
        result = repo.parent / "result.json"
        result.write_text(json.dumps({"summary": summary}))
        env["GOH_SHIP_RESULT"] = str(result)
    return _run(repo, "bash", "tools/ship_release.sh", version, env=env, check=check)


def _versions(path: Path) -> dict[str, str]:
    return dict(re.findall(r'name = "([^"]+)"\nversion = "([^"]+)"', path.read_text()))


def test_the_bump_reaches_the_manifest_and_both_lockfiles(tmp_path: Path) -> None:
    repo = _fixture(tmp_path)
    done = _kit(repo, "0.1.1", "A fix that ships")
    assert done.returncode == 0, done.stdout + done.stderr  # noqa: S101
    assert 'version = "0.1.1"' in (repo / "Cargo.toml").read_text()  # noqa: S101
    lock = _versions(repo / "Cargo.lock")
    assert [lock[n] for n in ("multitop", "multitop-agent", "multitop-testassert")] == ["0.1.1"] * 3  # noqa: S101
    assert lock["multitop-vault"] == "0.21.0", "the vault has its own version"  # noqa: S101
    assert lock["serde"] == "0.1.0", "a registry crate that happens to share the number is not ours"  # noqa: S101
    fuzz = _versions(repo / "fuzz" / "Cargo.lock")
    assert fuzz == {"multitop": "0.1.1", "multitop-agent": "0.1.1"}  # noqa: S101


def test_the_stanza_is_cut_committed_and_release_sh_runs_on_that_commit(tmp_path: Path) -> None:
    repo = _fixture(tmp_path)
    done = _kit(repo, "v0.1.1", "A fix that ships")
    assert done.returncode == 0, done.stdout + done.stderr  # noqa: S101
    log = (repo / "CHANGELOG.md").read_text()
    assert re.search(r"^## v0\.1\.1 — A fix that ships ", log, re.MULTILINE), log  # noqa: S101
    assert "## Unreleased\n\n## v0.1.1" in log, "Unreleased is left empty above the new stanza"  # noqa: S101
    assert (repo / "tools" / ".release-args").read_text().strip() == "v0.1.1"  # noqa: S101
    head = _run(repo, "git", "log", "-1", "--format=%s").stdout.strip()
    assert head.startswith("release v0.1.1: "), head  # noqa: S101
    assert _run(repo, "git", "status", "--porcelain", "--untracked-files=no").stdout == ""  # noqa: S101
    changed = _run(repo, "git", "show", "--name-only", "--format=", "HEAD").stdout.split()
    assert sorted(changed) == ["CHANGELOG.md", "Cargo.lock", "Cargo.toml", "fuzz/Cargo.lock"]  # noqa: S101


def test_a_dirty_tree_is_refused_before_anything_moves(tmp_path: Path) -> None:
    repo = _fixture(tmp_path)
    (repo / "Cargo.toml").write_text((repo / "Cargo.toml").read_text() + "\n# edit\n")
    done = _kit(repo, "0.1.1")
    assert done.returncode != 0  # noqa: S101
    assert "uncommitted" in done.stdout + done.stderr  # noqa: S101
    assert not (repo / "tools" / ".release-args").exists(), "release.sh must not run"  # noqa: S101


def test_a_tree_that_already_declares_another_new_version_is_refused_untouched(tmp_path: Path) -> None:
    repo = _fixture(tmp_path)
    _run(repo, "sed", "-i.bak", 's/^version = "0.1.0"/version = "0.3.0"/', "Cargo.toml")
    (repo / "Cargo.toml.bak").unlink()
    _run(repo, "git", "-c", "commit.gpgsign=false", "commit", "-q", "-am", "bump", "--no-verify")
    done = _kit(repo, "0.1.1")
    assert done.returncode != 0  # noqa: S101
    assert "goh ship --version 0.3.0" in done.stdout + done.stderr  # noqa: S101
    assert 'version = "0.3.0"' in (repo / "Cargo.toml").read_text()  # noqa: S101
    assert not (repo / "tools" / ".release-args").exists()  # noqa: S101


def test_an_empty_unreleased_refuses_and_leaves_the_checkout_as_it_was(tmp_path: Path) -> None:
    bare = CHANGELOG.replace(
        "### Fixed\n- The thing that was wrong is right now, so a release has something to say.\n\n", ""
    )
    repo = _fixture(tmp_path, changelog=bare)
    done = _kit(repo, "0.1.1")
    assert done.returncode != 0  # noqa: S101
    assert _run(repo, "git", "status", "--porcelain", "--untracked-files=no").stdout == ""  # noqa: S101
    assert 'version = "0.1.0"' in (repo / "Cargo.toml").read_text(), "the bump is rolled back"  # noqa: S101
    assert _versions(repo / "Cargo.lock")["multitop"] == "0.1.0"  # noqa: S101
    assert not (repo / "tools" / ".release-args").exists()  # noqa: S101


def test_an_already_bumped_tree_with_its_stanza_goes_straight_to_release_sh(tmp_path: Path) -> None:
    done_log = CHANGELOG.replace("## Unreleased\n", "## Unreleased\n\n## v0.1.1 — by hand _(2026-01-02)_\n- x.\n", 1)
    done_log = done_log.replace(
        "### Fixed\n- The thing that was wrong is right now, so a release has something to say.\n", ""
    )
    repo = _fixture(tmp_path, changelog=done_log)
    _run(repo, "sed", "-i.bak", 's/^version = "0.1.0"/version = "0.1.1"/', "Cargo.toml")
    (repo / "Cargo.toml.bak").unlink()
    _run(repo, "git", "-c", "commit.gpgsign=false", "commit", "-q", "-am", "bump", "--no-verify")
    before = _run(repo, "git", "rev-parse", "HEAD").stdout
    done = _kit(repo, "0.1.1")
    assert done.returncode == 0, done.stdout + done.stderr  # noqa: S101
    assert _run(repo, "git", "rev-parse", "HEAD").stdout == before, "no second commit"  # noqa: S101
    assert (repo / "tools" / ".release-args").read_text().strip() == "v0.1.1"  # noqa: S101


# -- the real tree -------------------------------------------------------------------------------


def _gatesrc_value(key: str) -> str:
    out = subprocess.run(  # noqa: S603
        ["/bin/bash", "-c", f'. ./.gatesrc >/dev/null 2>&1 </dev/null; printf %s "${{{key}-}}"'],
        cwd=REPO, capture_output=True, text=True, stdin=subprocess.DEVNULL, check=False,
    )  # fmt: skip
    return out.stdout


def test_gatesrc_declares_the_kit_the_changelog_contract_and_the_single_push() -> None:
    assert _gatesrc_value("GOH_SHIP_RELEASE") == "tools/ship_release.sh {version}"  # noqa: S101
    assert _gatesrc_value("GOH_SHIP_CHANGELOG") == "cut"  # noqa: S101
    assert _gatesrc_value("GOH_SHIP_RELEASE_PUSHES") == "1", (  # noqa: S101
        "release.sh installs and then pushes main and the tag once; a push step before it would run the "
        "push gate over a tree whose installed binary is stale (tools/installed_freshness.py)"
    )
    assert _gatesrc_value("GOH_INSTALLS") == "multitop"  # noqa: S101


def test_the_kit_is_executable_and_names_exactly_the_crates_that_follow_the_workspace_version() -> None:
    assert KIT.stat().st_mode & stat.S_IXUSR  # noqa: S101
    text = KIT.read_text()
    named = set(re.search(r'^WORKSPACE_CRATES="([^"]*)"', text, re.MULTILINE).group(1).split())
    follows = set()
    for manifest in (REPO / "crates").glob("*/Cargo.toml"):
        body = manifest.read_text()
        if re.search(r"^version\.workspace\s*=\s*true", body, re.MULTILINE):
            follows.add(re.search(r'^name = "([^"]+)"', body, re.MULTILINE).group(1))
    assert named == follows, f"the kit rewrites {sorted(named)}; the workspace version reaches {sorted(follows)}"  # noqa: S101


def test_the_real_lockfiles_agree_with_the_manifest_for_every_crate_the_kit_rewrites() -> None:
    version = re.search(r'\[workspace\.package\]\nversion = "([^"]+)"', (REPO / "Cargo.toml").read_text())[1]
    for lock in (REPO / "Cargo.lock", REPO / "fuzz" / "Cargo.lock"):
        found = _versions(lock)
        for name in ("multitop", "multitop-agent", "multitop-testassert"):
            if name in found:
                assert found[name] == version, f"{lock.relative_to(REPO)} names {name} {found[name]}, not {version}"  # noqa: S101
