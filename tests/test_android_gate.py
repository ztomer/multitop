"""tools/android_gate.sh refuses what it must, in both directions, on one machine.

The build-host policy (house rule: macOS is Apple silicon only, Linux is x86_64
or aarch64, 32-bit never) is a hard failure, never an arch fallback that builds
anyway; MONITOR_ANDROID_BUILD_OS/ARCH stand in for uname so the reject AND the
accept side are both exercised here. And a host without the Android SDK gets a
red gate that says how to install it -- never a silent skip.
"""

import os
import subprocess
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]
GATE = ROOT / "tools" / "android_gate.sh"


def _goh() -> str:
    for candidate in (os.environ.get("GOH_DIR"), ROOT / ".gates_of_heck", Path.home() / "Projects" / "gates_of_heck"):
        if candidate and (Path(candidate) / "tui" / "lib.sh").is_file():
            return str(candidate)
    raise AssertionError("no gates_of_heck checkout (GOH_DIR) -- the gate sources its tui/lib.sh")


def _run(*args: str, **env: str) -> subprocess.CompletedProcess:
    full = {**os.environ, "GOH_DIR": _goh(), "NO_COLOR": "1", **env}
    return subprocess.run(
        ["bash", str(GATE), *args],
        cwd=ROOT,
        env=full,
        capture_output=True,
        text=True,
        stdin=subprocess.DEVNULL,
        timeout=60,
        check=False,
    )


@pytest.mark.parametrize("os_name,arch", [("Darwin", "arm64"), ("Linux", "x86_64"), ("Linux", "aarch64")])
def test_a_supported_build_host_is_accepted(os_name, arch):
    proc = _run("--check-host", MONITOR_ANDROID_BUILD_OS=os_name, MONITOR_ANDROID_BUILD_ARCH=arch)
    assert proc.returncode == 0, proc.stdout + proc.stderr


@pytest.mark.parametrize(
    "os_name,arch",
    [
        ("Darwin", "x86_64"),  # Intel Mac: dropped
        ("Linux", "i686"),  # 32-bit: never
        ("Linux", "armv7l"),  # 32-bit: never
        ("MINGW64_NT-10.0", "x86_64"),
        ("FreeBSD", "amd64"),
    ],
)
def test_an_unsupported_build_host_fails_hard(os_name, arch):
    proc = _run("--check-host", MONITOR_ANDROID_BUILD_OS=os_name, MONITOR_ANDROID_BUILD_ARCH=arch)
    assert proc.returncode != 0
    assert f"unsupported build host {os_name}/{arch}" in proc.stdout + proc.stderr


def test_a_missing_sdk_is_a_red_gate_that_says_how_to_install_it(tmp_path):
    proc = _run(
        ANDROID_HOME=str(tmp_path / "no-sdk"),
        ANDROID_SDK_ROOT="",
        MONITOR_ANDROID_BUILD_OS="Linux",
        MONITOR_ANDROID_BUILD_ARCH="x86_64",
    )
    out = proc.stdout + proc.stderr
    assert proc.returncode != 0, out
    assert "no Android SDK" in out
    assert "sdkmanager" in out


def test_the_coverage_floor_is_one_whole_number_and_the_roadmap_records_it():
    floor = (ROOT / "tools" / "android_coverage_floor.txt").read_text().strip()
    assert floor.isdigit(), f"tools/android_coverage_floor.txt must hold one whole number, found {floor!r}"
    roadmap = (ROOT / "docs" / "roadmap.md").read_text()
    assert f"a Kotlin coverage floor of {floor}%" in roadmap, (
        "docs/roadmap.md A2.2 does not record the floor in tools/android_coverage_floor.txt"
    )
