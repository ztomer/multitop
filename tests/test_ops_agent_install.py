"""The Ops agent cannot be installed with a rate this repo carries.

servers ROADMAP E53: the installed agent's tick was a number in the install
template (`StartInterval 390`), and launchd fixes an interval when the job is
loaded - so the tick could not follow the estate. When the estate's
publication moved from every 15 minutes to every 5, the agent kept reading
every 6.5 minutes: slower than the publication, so its verdict could be two
publications behind, and the client's own warning fired on every tick with
nothing to do about it but reinstall the agent.

The tick is now READ by the agent itself, from the cadence the estate
publishes with its verdict (DECISIONS D15). So the template must not be able
to carry a rate at all:

  * the launchd job it writes has no interval in it - it keeps the agent
    ALIVE, and nothing else;
  * the operator cannot hand it a rate either: `--interval` is refused;
  * the wrapper runs the agent in its loop, not one pass and out.

Each of these is checked against the template the script would install - the
script is run, in a scratch HOME, and what it writes is read - not against a
copy of it kept here, which would only prove the copy is right.
"""

from __future__ import annotations

import os
import plistlib
import subprocess
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
SCRIPT = REPO / "tools" / "install_ops_poller.sh"


def _run(tmp_path: Path, *args: str) -> subprocess.CompletedProcess:
    """Run the install template with a scratch HOME, a fake multitop and a
    fake launchctl.

    The script resolves the binary with `command -v multitop`, so the fake
    one on PATH is what the wrapper would point at. The fake `launchctl`
    matters just as much: the script installs a LAUNCHD JOB, and the label
    is the operator's own - so without the fake, running this test would
    unload the operator's live agent and replace it with one pointing into
    this test's scratch directory. The fake records what the script asked
    launchd to do and does none of it.
    """
    home = tmp_path / "home"
    (home / "Library" / "LaunchAgents").mkdir(parents=True)
    bindir = tmp_path / "bin"
    bindir.mkdir()
    fake = bindir / "multitop"
    fake.write_text("#!/bin/sh\nexit 0\n")
    fake.chmod(0o755)
    launcher = bindir / "launchctl"
    launcher.write_text(
        "#!/bin/sh\n"
        f"printf '%s\\n' \"$*\" >> '{tmp_path / 'launchctl.log'}'\n"
        "exit 0\n"
    )
    launcher.chmod(0o755)
    env = dict(os.environ)
    env["HOME"] = str(home)
    env["PATH"] = f"{bindir}{os.pathsep}{env['PATH']}"
    return subprocess.run(
        ["bash", str(SCRIPT), *args],
        capture_output=True,
        text=True,
        env=env,
        stdin=subprocess.DEVNULL,
        check=False,
    )


def test_the_launchd_job_carries_no_rate(tmp_path):
    """The plist has no interval: the agent's tick is not in this repo."""
    done = _run(tmp_path)
    assert done.returncode == 0, done.stderr
    plist_path = tmp_path / "home" / "Library" / "LaunchAgents" / "com.ztomer.multitop.ops-poll.plist"
    assert plist_path.exists(), f"the template wrote no plist: {done.stdout} {done.stderr}"
    job = plistlib.loads(plist_path.read_bytes())
    assert "StartInterval" not in job, "the launchd job carries a rate: the tick is fixed at install time"
    assert job.get("KeepAlive"), "the launchd job does not keep the agent alive"
    assert job.get("RunAtLoad") is True, "the agent does not start when the job is loaded"


def test_the_operator_cannot_hand_the_agent_a_rate(tmp_path):
    """`--interval` is refused: no rate can be installed from this repo."""
    done = _run(tmp_path, "--interval", "60")
    assert done.returncode != 0, "the template accepted a rate"
    said = (done.stdout + done.stderr).lower()
    assert "interval" in said, f"the refusal does not say what was refused: {done.stdout} {done.stderr}"


def test_the_job_is_loaded_into_launchd(tmp_path):
    """The template installs a JOB: it is bootstrapped, not just written."""
    done = _run(tmp_path)
    assert done.returncode == 0, done.stderr
    asked = (tmp_path / "launchctl.log").read_text()
    assert "bootstrap" in asked, f"the job was never loaded: {asked}"
    assert "bootout" in asked, f"the old job was never unloaded: {asked}"


def test_the_wrapper_runs_the_agent_in_its_loop(tmp_path):
    """The wrapper runs the agent, which paces itself - not one pass."""
    done = _run(tmp_path)
    assert done.returncode == 0, done.stderr
    runner = tmp_path / "home" / "Library" / "Application Support" / "multitop" / "ops-poll-agent.sh"
    assert runner.exists(), f"the template wrote no wrapper: {done.stdout} {done.stderr}"
    said = runner.read_text()
    assert "--ops-agent" in said, "the wrapper runs a single pass, so nothing paces the tick"
    assert "--ops-poll" not in said, "the wrapper runs the one-pass mode"


def test_the_template_carries_no_rate_to_install(tmp_path):
    """The template itself holds no rate: the number is gone, not moved."""
    said = SCRIPT.read_text()
    assert "<key>StartInterval</key>" not in said, "the template still writes an interval into the job"
    assert "interval=" not in said, "the template still carries a rate in a variable"
