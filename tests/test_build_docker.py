"""build.sh's docker backend keeps its cargo target OFF the host mount and names its image.

EST.HOSTMOUNTTARGET (2026-11-13). DV.LINTEXEC found a build script on a colima virtiofs bind
mount losing its exec bit intermittently ("Permission denied" running build-script-build) and
moved divoom's target dir to a docker NAMED volume. build.sh's docker backend had the same shape:
`-v $PWD:/src` read-write with CARGO_TARGET_DIR=/src/target/docker, on the moving tag rust:alpine.
Now the target and the cargo registry live in named volumes, the source is mounted read-only, the
image is a named release, and only the finished agent is copied out through a mount of the one
output directory (where release.sh and tools/check_agent_version.py read it).
"""

from __future__ import annotations

import re
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
BUILD = ROOT / "build.sh"
TARGET = "x86_64-unknown-linux-musl"


def _docker_backend() -> str:
    """The marked docker-backend block of build.sh: its pins and build_agent_docker."""
    text = BUILD.read_text()
    m = re.search(
        r"^# >>> docker backend\n(.*?)^# <<< docker backend\n",
        text,
        re.DOTALL | re.MULTILINE,
    )
    assert m, "build.sh must mark its docker backend with '# >>> docker backend' ... '# <<< docker backend'"  # noqa: S101
    return m.group(1)


def _run_backend(tmp_path: Path) -> list[str]:
    """Run build_agent_docker against a fake docker; return the argv of its `docker run`."""
    log = tmp_path / "docker.log"
    fake = tmp_path / "docker"
    fake.write_text(f'#!/usr/bin/env bash\necho "$*" >> "{log}"\n')
    fake.chmod(0o755)
    work = tmp_path / "repo"
    work.mkdir()
    script = (
        f'set -euo pipefail\nPROFILE=release\nCARGO_PROFILE_FLAG=--release\ncd "{work}"\n'
        f'PATH="{tmp_path}:$PATH"\n{_docker_backend()}\nbuild_agent_docker {TARGET}\n'
    )
    proc = subprocess.run(  # noqa: S603
        ["bash", "-c", script],  # noqa: S607
        capture_output=True,
        text=True,
        stdin=subprocess.DEVNULL,
        timeout=10,
        check=False,
    )
    assert proc.returncode == 0, proc.stdout + proc.stderr  # noqa: S101
    # The container script spans lines; one `docker run` per call, so the whole log is its argv.
    text = log.read_text()
    return [" ".join(text.split())] if text.startswith("run ") else []


def test_image_is_a_named_release(tmp_path: Path) -> None:
    """`rust:alpine` floats; the image names a compiler release AND an alpine release."""
    (run,) = _run_backend(tmp_path)
    image = re.search(r"\brust:(\S+)", run)
    assert image, run  # noqa: S101
    assert re.fullmatch(r"\d+\.\d+\.\d+-alpine\d+\.\d+", image.group(1)), f"moving tag: {image.group(0)}"  # noqa: S101
    assert "RUSTUP_TOOLCHAIN=" + image.group(1).split("-")[0] in run, "the pin must override rust-toolchain.toml"  # noqa: S101


def test_cargo_target_is_a_named_volume_not_the_host_mount(tmp_path: Path) -> None:
    """The class: no cargo target dir on a bind mount of the host checkout."""
    (run,) = _run_backend(tmp_path)
    assert re.search(r"-v [a-z][\w.-]*:/cache/target\b", run), run  # noqa: S101
    assert "CARGO_TARGET_DIR=/cache/target" in run, run  # noqa: S101
    assert "/src/target" not in run, run  # noqa: S101
    # The source is read-only: nothing the build does can write the checkout.
    assert re.search(r"-v \S+:/src:ro\b", run), run  # noqa: S101
    # The only other host mount is the one output directory the finished agent is copied to.
    host_mounts = re.findall(r"-v (/\S+?):(\S+)", run)
    assert sorted(dst for _src, dst in host_mounts) == ["/out", "/src:ro"], host_mounts  # noqa: S101
    out_src = next(src for src, dst in host_mounts if dst == "/out")
    assert out_src.endswith("/target/docker"), out_src  # noqa: S101


def test_agent_is_copied_where_the_release_kit_reads_it(tmp_path: Path) -> None:
    """release.sh / check_agent_version.py read target/docker/<triple>/<profile>/multitop-agent."""
    (run,) = _run_backend(tmp_path)
    assert f"/out/{TARGET}/release/multitop-agent" in run, run  # noqa: S101


def test_no_host_mounted_cargo_target_anywhere_in_build_sh() -> None:
    """Grep proof, kept: neither the old read-write source mount nor the old moving tag returns."""
    text = BUILD.read_text()
    assert "rust:alpine" not in re.sub(r"(?m)^\s*#.*$", "", text), "moving tag rust:alpine"  # noqa: S101
    assert not re.search(r'-v "\$PWD":/src\b(?!:ro)', text), "read-write source bind mount"  # noqa: S101
    assert "CARGO_TARGET_DIR=/src" not in text, "cargo target on the host mount"  # noqa: S101
