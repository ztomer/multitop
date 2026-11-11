"""The gate on a GitHub-hosted Linux runner (CI.PUBLIC2): each step runs there, or says why not.

multitop's gate run 42926394892 (ubuntu) was red on steps written for the owner's macOS machine:

* tools/bench_check.py counts instructions with macOS's `/usr/bin/time -l`; GNU time has no `-l`.
  On a hosted non-macOS run it says NOT APPLICABLE (exit 78, gates_of_heck lib/not_applicable.py);
  anywhere else off macOS it fails closed, as it always did.
* tools/installed_freshness.py crashed on a machine with no `brew` instead of finding nothing.
* tools/lint_linux.sh pinned its container to linux/arm64, which an x86_64 host cannot exec; the
  platform is now the host's architecture (`ARCH` seam), and a 32-bit host is refused.
"""

from __future__ import annotations

import os
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
NA_EXIT = 78


def _env(**extra: str) -> dict[str, str]:
    env = {k: v for k, v in os.environ.items() if k not in ("GITHUB_ACTIONS", "OS", "ARCH")}
    env.update(extra)
    return env


def _bench(env: dict[str, str]) -> subprocess.CompletedProcess[str]:
    return subprocess.run(  # noqa: S603
        [sys.executable, str(ROOT / "tools" / "bench_check.py")],
        env=env,
        capture_output=True,
        text=True,
        stdin=subprocess.DEVNULL,
        timeout=60,
        check=False,
    )


def test_bench_off_macos_on_a_hosted_run_is_not_applicable() -> None:
    got = _bench(_env(OS="Linux", GITHUB_ACTIONS="true"))
    assert got.returncode == NA_EXIT, got.stdout + got.stderr  # noqa: S101
    assert "NOT APPLICABLE on this hosted runner" in got.stdout + got.stderr  # noqa: S101


def test_bench_off_macos_anywhere_else_fails_closed() -> None:
    got = _bench(_env(OS="Linux"))
    assert got.returncode == 1, got.stdout + got.stderr  # noqa: S101
    assert "NOT APPLICABLE" not in got.stdout + got.stderr  # noqa: S101
    assert "macOS" in got.stdout + got.stderr  # noqa: S101


def test_installed_binary_without_brew_is_nothing_installed(tmp_path: Path) -> None:
    code = "import sys; sys.path.insert(0, 'tools'); import installed_freshness as f; print(f.installed_binary())"
    env = _env(PATH=str(tmp_path))
    env.pop("MULTITOP_INSTALLED", None)
    got = subprocess.run(  # noqa: S603
        [sys.executable, "-c", code],
        cwd=ROOT,
        env=env,
        capture_output=True,
        text=True,
        stdin=subprocess.DEVNULL,
        timeout=60,
        check=False,
    )
    assert got.returncode == 0, got.stderr  # noqa: S101
    assert got.stdout.strip() == "None", got.stdout  # noqa: S101


def _lint_platform(tmp_path: Path, arch: str) -> subprocess.CompletedProcess[str]:
    log = tmp_path / "docker.log"
    docker = tmp_path / "docker"
    docker.write_text(f'#!/usr/bin/env bash\necho "$*" >> "{log}"\nexit 0\n')
    docker.chmod(0o755)
    goh = os.environ.get("GOH_DIR") or str(Path.home() / "Projects" / "gates_of_heck")
    env = _env(
        DOCKER=str(docker),
        COLIMA=str(docker),
        GOH_DIR=goh,
        ARCH=arch,
        NO_COLOR="1",
        GOH_CONTAINER_STATE=str(tmp_path / "state"),
    )
    return subprocess.run(  # noqa: S603
        ["bash", str(ROOT / "tools" / "lint_linux.sh")],  # noqa: S607
        env=env,
        cwd=ROOT,
        capture_output=True,
        text=True,
        stdin=subprocess.DEVNULL,
        timeout=30,
        check=False,
    )


def _runs(tmp_path: Path) -> list[str]:
    log = tmp_path / "docker.log"
    return [ln for ln in log.read_text().splitlines() if ln.startswith("run ")] if log.exists() else []


def test_lint_linux_runs_the_hosts_architecture(tmp_path: Path) -> None:
    for arch, platform in (("x86_64", "linux/amd64"), ("arm64", "linux/arm64"), ("aarch64", "linux/arm64")):
        case = tmp_path / arch
        case.mkdir()
        got = _lint_platform(case, arch)
        assert got.returncode == 0, got.stdout + got.stderr  # noqa: S101
        runs = _runs(case)
        assert len(runs) == 1 and f"--platform {platform} " in runs[0], runs  # noqa: S101


def test_lint_linux_refuses_a_32_bit_host(tmp_path: Path) -> None:
    got = _lint_platform(tmp_path, "i686")
    assert got.returncode != 0, got.stdout + got.stderr  # noqa: S101
    assert "i686" in got.stdout + got.stderr and not _runs(tmp_path)  # noqa: S101
