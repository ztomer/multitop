"""tools/lint_linux.sh starts its container runtime when down, and stops what it started.

Owner ruling D.MON2 (2026-10-31): on macOS the container runtime is Colima.
When `docker info` succeeds, use it and stop nothing. When it fails, run `colima start`,
run the lint, and ensure `colima stop` runs on every exit path (clean exit, lint failure,
or signal). Overridable DOCKER and COLIMA environment variables allow fakes under test; the
runtime seam itself is goh's lib/container_runtime.sh (GOH.CR1).
"""

from __future__ import annotations

import os
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
LINT_LINUX = ROOT / "tools" / "lint_linux.sh"
LINT_FAIL_EXIT = 101


def _goh() -> str:
    for candidate in (os.environ.get("GOH_DIR"), ROOT / ".gates_of_heck", Path.home() / "Projects" / "gates_of_heck"):
        if candidate and (Path(candidate) / "tui" / "lib.sh").is_file():
            return str(candidate)
    msg = "no gates_of_heck checkout (GOH_DIR) -- the gate sources its tui/lib.sh"
    raise AssertionError(msg)


def _setup(
    tmp_path: Path,
    *,
    info_fail: bool = False,
    run_fail: bool = False,
    run_sigint: bool = False,
) -> tuple[dict[str, str], Path, Path]:
    bin_dir = tmp_path / "bin"
    bin_dir.mkdir()

    colima_log = tmp_path / "colima.log"
    docker_log = tmp_path / "docker.log"

    fake_colima = bin_dir / "colima"
    fake_colima.write_text(
        f"""#!/usr/bin/env bash
set -e
echo "$1" >> "{colima_log}"
case "$1" in
  start) : > "{tmp_path}/up" ;;
  stop) rm -f "{tmp_path}/up" ;;
esac
exit 0
"""
    )
    fake_colima.chmod(0o755)

    fake_docker = bin_dir / "docker"
    info_exit = "1" if info_fail else "0"
    run_fail_str = "1" if run_fail else "0"
    run_sigint_str = "1" if run_sigint else "0"

    fake_docker.write_text(
        f"""#!/usr/bin/env bash
set -e
echo "$1" >> "{docker_log}"
case "$1" in
  info)
    [ -f "{tmp_path}/up" ] && exit 0
    exit {info_exit}
    ;;
  run)
    if [ "{run_fail_str}" = "1" ]; then
      exit {LINT_FAIL_EXIT}
    fi
    if [ "{run_sigint_str}" = "1" ]; then
      kill -INT "$PPID"
      exit 130
    fi
    exit 0
    ;;
  *)
    exit 0
    ;;
esac
"""
    )
    fake_docker.chmod(0o755)

    env = {
        **os.environ,
        "DOCKER": str(fake_docker),
        "COLIMA": str(fake_colima),
        "GOH_DIR": _goh(),
        "NO_COLOR": "1",
        # The leases live here, never in the machine's real state dir.
        "GOH_CONTAINER_STATE": str(tmp_path / "runtime_state"),
    }
    return env, colima_log, docker_log


def _run_lint(env: dict[str, str]) -> subprocess.CompletedProcess[str]:
    return subprocess.run(  # noqa: S603
        ["bash", str(LINT_LINUX)],  # noqa: S607
        env=env,
        cwd=ROOT,
        capture_output=True,
        text=True,
        stdin=subprocess.DEVNULL,
        timeout=10,
        check=False,
    )


def test_runtime_already_up_never_starts_or_stops_colima(tmp_path: Path) -> None:
    env, colima_log, docker_log = _setup(tmp_path, info_fail=False)
    proc = _run_lint(env)
    assert proc.returncode == 0, proc.stdout + proc.stderr  # noqa: S101
    assert not colima_log.exists() or colima_log.read_text().strip() == ""  # noqa: S101
    assert "info" in docker_log.read_text()  # noqa: S101
    assert "run" in docker_log.read_text()  # noqa: S101


def test_runtime_down_starts_colima_and_stops_on_clean_exit(tmp_path: Path) -> None:
    env, colima_log, docker_log = _setup(tmp_path, info_fail=True)
    proc = _run_lint(env)
    assert proc.returncode == 0, proc.stdout + proc.stderr  # noqa: S101
    assert colima_log.read_text().splitlines() == ["start", "stop"]  # noqa: S101
    assert "info" in docker_log.read_text()  # noqa: S101
    assert "run" in docker_log.read_text()  # noqa: S101


def test_runtime_down_stops_colima_when_lint_fails(tmp_path: Path) -> None:
    env, colima_log, docker_log = _setup(tmp_path, info_fail=True, run_fail=True)
    proc = _run_lint(env)
    assert proc.returncode == LINT_FAIL_EXIT, proc.stdout + proc.stderr  # noqa: S101
    assert colima_log.read_text().splitlines() == ["start", "stop"]  # noqa: S101
    assert "info" in docker_log.read_text()  # noqa: S101
    assert "run" in docker_log.read_text()  # noqa: S101


def test_runtime_down_stops_colima_on_sigint(tmp_path: Path) -> None:
    env, colima_log, _docker_log = _setup(tmp_path, info_fail=True, run_sigint=True)
    proc = _run_lint(env)
    assert proc.returncode in (130, -2), proc.stdout + proc.stderr  # noqa: S101
    assert colima_log.read_text().splitlines() == ["start", "stop"]  # noqa: S101
