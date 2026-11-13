"""tools/lint_linux.sh starts its container runtime when down, and stops what it started.

Owner ruling D.MON2 (2026-10-31): on macOS the container runtime is Colima.
When `docker info` succeeds, use it and stop nothing. When it fails, run `colima start`,
run the lint, and ensure `colima stop` runs on every exit path (clean exit, lint failure,
or signal). Overridable DOCKER and COLIMA environment variables allow fakes under test; the
runtime seam itself is goh's lib/container_runtime.sh (GOH.CR1).
"""

from __future__ import annotations

import os
import re
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
    run_mode: str = "ok",
    image_missing: bool = False,
    build_fail: bool = False,
) -> tuple[dict[str, str], Path, Path]:
    """`run_mode` is what the fake container does: ok, fail (exit 101 + an I/O error on stderr),
    sigint, silent_ioerror (the verdict is LOST: exit 0, an I/O error, no sentinel) or
    lying_toolchain (exit 0, clippy "ran" on a rustc other than the pinned one)."""
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
    image_exit = "1" if image_missing else "0"
    build_exit = "1" if build_fail else "0"

    fake_docker.write_text(
        f"""#!/usr/bin/env bash
set -e
echo "$*" >> "{docker_log}"
case "$1" in
  info)
    [ -f "{tmp_path}/up" ] && exit 0
    exit {info_exit}
    ;;
  image)
    exit {image_exit}
    ;;
  build)
    cat >/dev/null
    if [ "{build_exit}" != "0" ]; then
      echo "failed to write /var/lib/containerd/meta.db: input/output error" >&2
      exit 1
    fi
    exit 0
    ;;
  run)
    want=""
    for a in "$@"; do case "$a" in RUSTUP_TOOLCHAIN=*) want="${{a#RUSTUP_TOOLCHAIN=}}" ;; esac; done
    case "{run_mode}" in
      sigint)
        kill -INT "$PPID"
        exit 130
        ;;
      fail)
        echo "error: could not write /root/.rustup/tmp: input/output error (os error 5)" >&2
        exit {LINT_FAIL_EXIT}
        ;;
      silent_ioerror)
        echo "error: could not write /root/.rustup/tmp: input/output error (os error 5)" >&2
        exit 0
        ;;
      lying_toolchain)
        echo "MONITOR_LINT_LINUX_OK rustc 1.99.1 (fake)"
        exit 0
        ;;
      *)
        echo "MONITOR_LINT_LINUX_OK rustc $want (fake)"
        exit 0
        ;;
    esac
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
    env, colima_log, docker_log = _setup(tmp_path, info_fail=True, run_mode="fail")
    proc = _run_lint(env)
    assert proc.returncode != 0, proc.stdout + proc.stderr  # noqa: S101
    assert colima_log.read_text().splitlines() == ["start", "stop"]  # noqa: S101
    assert "info" in docker_log.read_text()  # noqa: S101
    assert "run" in docker_log.read_text()  # noqa: S101


def test_runtime_down_stops_colima_on_sigint(tmp_path: Path) -> None:
    env, colima_log, _docker_log = _setup(tmp_path, info_fail=True, run_mode="sigint")
    proc = _run_lint(env)
    assert proc.returncode in (130, -2), proc.stdout + proc.stderr  # noqa: S101
    assert colima_log.read_text().splitlines() == ["start", "stop"]  # noqa: S101


def _failed(proc: subprocess.CompletedProcess[str]) -> None:
    out = proc.stdout + proc.stderr
    assert proc.returncode != 0, out  # noqa: S101
    assert "FAILED" in out, out  # noqa: S101
    assert "input/output error" in out, "the container's own words must reach the reader\n" + out  # noqa: S101


def test_failing_container_reads_failed_with_its_own_error(tmp_path: Path) -> None:
    """MON.LINTLINUX: a container that exits non-zero names FAILED and shows why."""
    env, colima_log, _ = _setup(tmp_path, info_fail=True, run_mode="fail")
    proc = _run_lint(env)
    _failed(proc)
    assert colima_log.read_text().splitlines() == ["start", "stop"]  # noqa: S101


def test_container_that_lost_its_verdict_is_not_a_pass(tmp_path: Path) -> None:
    """Exit 0 plus an I/O error and no proof clippy ran (the 2026-11-13 full-disk run read
    PASSED) is FAILED: a pass needs the sentinel the container prints only after clippy
    succeeded, never the exit status alone."""
    env, colima_log, _ = _setup(tmp_path, info_fail=True, run_mode="silent_ioerror")
    proc = _run_lint(env)
    _failed(proc)
    assert colima_log.read_text().splitlines() == ["start", "stop"]  # noqa: S101


def test_toolchain_that_is_not_the_pinned_one_is_not_a_pass(tmp_path: Path) -> None:
    """The image tag may not lie: clippy that ran on another rustc than the pinned one fails."""
    env, _, _ = _setup(tmp_path, run_mode="lying_toolchain")
    proc = _run_lint(env)
    out = proc.stdout + proc.stderr
    assert proc.returncode != 0, out  # noqa: S101
    assert "FAILED" in out, out  # noqa: S101
    assert "1.99.1" in out, out  # noqa: S101


def test_image_build_failure_fails_the_step(tmp_path: Path) -> None:
    env, colima_log, _ = _setup(tmp_path, info_fail=True, image_missing=True, build_fail=True)
    proc = _run_lint(env)
    _failed(proc)
    assert colima_log.read_text().splitlines() == ["start", "stop"]  # noqa: S101


def _pinned() -> tuple[str, str]:
    text = LINT_LINUX.read_text()
    version = re.search(r'^RUST_VERSION="\$\{MONITOR_LINT_RUST_VERSION:-([^}]+)\}"', text, re.MULTILINE)
    debian = re.search(r"^DEBIAN=([a-z]+)$", text, re.MULTILINE)
    assert version and debian, "lint_linux.sh must pin RUST_VERSION and DEBIAN on their own lines"  # noqa: S101
    return version.group(1), debian.group(1)


def test_image_is_a_named_release() -> None:
    """The lint image is a named release, never a moving tag (house rule: cutting edge)."""
    version, _debian = _pinned()
    assert re.fullmatch(r"\d+\.\d+\.\d+", version), f"moving toolchain {version}"  # noqa: S101
    text = LINT_LINUX.read_text()
    assert "rust:$RUST_VERSION-$DEBIAN" in text, "the base image must be built from the pin"  # noqa: S101
    assert not re.search(r"rust:(latest|stable|slim|\d+(-|\b(?![.])))", text), "moving rust tag"  # noqa: S101


def test_image_tag_is_the_toolchain_the_container_runs(tmp_path: Path) -> None:
    """Image tag == toolchain: the same pin names the image and RUSTUP_TOOLCHAIN, which overrides
    rust-toolchain.toml's floating `stable` so the container does not fetch another compiler at
    run time (it fetched 1.100 on a rust:1.99.1 image). The cargo target is a NAMED volume."""
    version, debian = _pinned()
    env, _, docker_log = _setup(tmp_path)
    proc = _run_lint(env)
    assert proc.returncode == 0, proc.stdout + proc.stderr  # noqa: S101
    run = next(line for line in docker_log.read_text().splitlines() if line.startswith("run "))
    assert f"RUSTUP_TOOLCHAIN={version}" in run, run  # noqa: S101
    assert f"monitor-linux-lint:{version}-{debian}" in run, run  # noqa: S101
    assert "monitor-linux-lint-target:/cache/target" in run, run  # noqa: S101
