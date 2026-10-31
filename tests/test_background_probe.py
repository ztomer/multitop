"""The OSC 11 background probe, against a terminal that answers.

Until 2026-10-10 the probe read the reply through crossterm's KEY parser, which
turns `ESC ]` into an Alt+`]` key and drops the ESC both terminators need, so
the reply was never recognised and every probe timed out. Every unit test passed:
they fed `parse_osc11_reply` a string, and the string was never what the real
reader produced. Only a real pty that answers can see that, so this is one --
the binary, on a pty, with `--print-background`, the app's own answer.
"""

import os
import pty
import select
import sys
import time

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import pytest
from tmux_harness import find_binary

BINARY = find_binary()


def _run(reply):
    """`multitop --print-background` on a pty that sends `reply` when asked."""
    pid, fd = pty.fork()
    if pid == 0:
        env = {k: v for k, v in os.environ.items() if k != "COLORFGBG"}
        os.execve(BINARY, [BINARY, "--print-background"], env)
    out, answered, end = b"", False, time.time() + 5
    while time.time() < end:
        ready, _, _ = select.select([fd], [], [], 0.005)
        if not ready:
            continue
        try:
            chunk = os.read(fd, 4096)
        except OSError:
            break
        if not chunk:
            break
        out += chunk
        if reply is not None and not answered and b"\x1b]11;?" in out:
            os.write(fd, reply)
            answered = True
    os.waitpid(pid, 0)
    return out.decode(errors="replace"), answered


@pytest.mark.skipif(BINARY is None, reason="no multitop binary in this build tree")
@pytest.mark.parametrize(
    "reply",
    [b"\x1b]11;rgb:ffff/ffff/ffff\x1b\\", b"\x1b]11;rgb:ffff/ffff/ffff\x07"],
    ids=["ST", "BEL"],
)
def test_a_terminal_that_answers_is_heard(reply):
    out, answered = _run(reply)
    assert answered, f"the probe never asked: {out!r}"
    assert "source: OSC 11" in out, out
    assert "r: 255, g: 255, b: 255" in out, out


@pytest.mark.skipif(BINARY is None, reason="no multitop binary in this build tree")
def test_a_silent_terminal_costs_one_bounded_wait():
    started = time.time()
    out, _ = _run(None)
    assert "source: unknown" in out, out
    assert time.time() - started < 3, "the probe did not give up on a silent terminal"
