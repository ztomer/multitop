"""L3 -- the exec channel against live hosts, with `ls` as the oracle.

The question these answer is the one the unit tests cannot: does what reaches
the client equal what the command actually printed, on a real host, over a real
SSH connection? And -- the point of the whole change -- does it stay equal when
the thing that used to decide the stream's shape changes underneath it?

`ls` rather than an upgrade command on purpose. Its output is knowable in
advance, it changes nothing, and it is safe to run a hundred times.

# On the oracle

`/bin/ls -1`, absolutely and explicitly single-column, on both sides.

The first version of this compared the channel's output against `ssh host ls`
and failed on every host. The channel was right and the oracle was wrong: over
the exec channel `ls` resolves the operator's shell alias and finds `isatty(1)`
true, so it printed three columns of coloured, icon-decorated names; the oracle
had neither a tty nor the alias and printed 235 plain lines. Two different
commands, compared, and found to differ.

An oracle has to hold everything constant except the thing under test. The
absolute path defeats the alias on both sides and `-1` defeats the tty layout,
which leaves exactly one variable: the transport. That the aliased, coloured,
multi-column output happens at all is the pty doing its job -- it is asserted
separately, in `an_alias_and_a_terminal_reach_the_command`.
"""

import os
import shutil
import subprocess
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import exec_wire

CONTROL_PATH = "~/.ssh/multitop-live-test-%C"
SSH_BASE = [
    "ssh", "-o", "BatchMode=yes", "-o", "ControlMaster=auto",
    "-o", f"ControlPath={CONTROL_PATH}", "-o", "ControlPersist=30s",
    "-o", "ConnectTimeout=10", "-T",
]
ORACLE_CMD = "/bin/ls -1 /etc"
REMOTE_AGENT = "/tmp/multitop-agent-livetest"

#: Hosts to exercise, as `user@host:arch`. Overridable so this is not pinned to
#: one person's network; skipped with a stated reason when unset and
#: unreachable, never quietly passed.
DEFAULT_HOSTS = "ztomer@192.168.0.33:x86_64,ztomer@192.168.0.90:x86_64,ztomer@192.168.0.158:aarch64"


def hosts():
    raw = os.environ.get("MULTITOP_LIVE_HOSTS", DEFAULT_HOSTS)
    out = []
    for entry in filter(None, (e.strip() for e in raw.split(","))):
        target, _, arch = entry.partition(":")
        out.append((target, arch or "x86_64"))
    return out


def agent_for(arch):
    """The cross-compiled agent for `arch`, or None if it was never built."""
    root = os.environ.get("CARGO_TARGET_DIR") or os.path.expanduser("~/.cache/cargo-target")
    path = os.path.join(root, f"{arch}-unknown-linux-musl", "release", "multitop-agent")
    return path if os.path.isfile(path) else None


def reachable(target):
    try:
        return subprocess.run(
            SSH_BASE + [target, "true"], capture_output=True, timeout=20
        ).returncode == 0
    except (subprocess.TimeoutExpired, OSError):
        return False


def control_path_for(target):
    """Ask ssh itself where the socket for this target would live."""
    out = subprocess.run(
        SSH_BASE + ["-G", target], capture_output=True, text=True, timeout=20
    ).stdout
    for line in out.splitlines():
        if line.startswith("controlpath "):
            return os.path.expanduser(line.split(" ", 1)[1].strip())
    return None


class LiveExecChannel(unittest.TestCase):
    """One prepared host per entry in MULTITOP_LIVE_HOSTS."""

    prepared = []
    skip_reason = None

    @classmethod
    def setUpClass(cls):
        if not shutil.which("ssh"):
            cls.skip_reason = "no ssh on PATH"
            return
        missing_arch, unreachable_hosts = set(), []
        for target, arch in hosts():
            binary = agent_for(arch)
            if binary is None:
                missing_arch.add(arch)
                continue
            if not reachable(target):
                unreachable_hosts.append(target)
                continue
            up = subprocess.run(
                ["scp", "-q", "-o", "BatchMode=yes", binary, f"{target}:{REMOTE_AGENT}"],
                capture_output=True, timeout=180,
            )
            if up.returncode != 0:
                unreachable_hosts.append(f"{target} (upload: {up.stderr.decode().strip()})")
                continue
            subprocess.run(SSH_BASE + [target, f"chmod 755 {REMOTE_AGENT}"], timeout=30)
            cls.prepared.append((target, arch))
        if not cls.prepared:
            parts = []
            if missing_arch:
                parts.append(
                    "no agent built for " + ", ".join(sorted(missing_arch))
                    + " (run ./build.sh)"
                )
            if unreachable_hosts:
                parts.append("unreachable: " + ", ".join(unreachable_hosts))
            cls.skip_reason = "; ".join(parts) or "no live hosts configured"

    @classmethod
    def tearDownClass(cls):
        for target, _ in cls.prepared:
            subprocess.run(SSH_BASE + [target, f"rm -f {REMOTE_AGENT}"], timeout=30)
        cls.clear_sockets()

    @classmethod
    def clear_sockets(cls):
        for target, _ in cls.prepared:
            path = control_path_for(target)
            if path and os.path.exists(path):
                os.remove(path)

    def setUp(self):
        if self.skip_reason:
            self.skipTest(self.skip_reason)

    def run_exec(self, target, command, cols=80, rows=24):
        proc = subprocess.run(
            SSH_BASE + [target, f"{REMOTE_AGENT} exec"],
            input=exec_wire.request(command, cols=cols, rows=rows),
            capture_output=True, timeout=120,
        )
        return exec_wire.decode(proc.stdout)

    def oracle(self, target):
        proc = subprocess.run(
            SSH_BASE + [target, ORACLE_CMD], capture_output=True, timeout=120
        )
        return proc.stdout.decode()

    def assert_matches_oracle(self, target, note):
        frames = self.run_exec(target, ORACLE_CMD)
        got = exec_wire.output(frames).replace(b"\r\n", b"\n").decode()
        want = self.oracle(target)
        self.assertEqual(
            exec_wire.exit_code(frames), 0, f"{target} {note}: no clean Exit frame"
        )
        # Stated before the equality check, because "identical" is the headline
        # but "nothing appeared twice" is the defect being guarded.
        for line in set(got.splitlines()):
            self.assertLessEqual(
                got.splitlines().count(line),
                want.splitlines().count(line),
                f"{target} {note}: {line!r} appeared more often than the host printed it",
            )
        self.assertEqual(got, want, f"{target} {note}: output differs from the oracle")

    def test_01_output_matches_the_host_with_a_cold_socket(self):
        """No ControlMaster socket: ssh opens its own connection."""
        self.clear_sockets()
        for target, arch in self.prepared:
            with self.subTest(host=target, arch=arch):
                self.assert_matches_oracle(target, "cold")

    def test_02_output_matches_the_host_with_a_warm_socket(self):
        """A socket left live by the previous test: ssh multiplexes over it."""
        for target, arch in self.prepared:
            with self.subTest(host=target, arch=arch):
                self.assert_matches_oracle(target, "warm")

    def test_03_output_matches_with_multiplexing_disabled(self):
        """A plain file where the socket goes.

        ssh prints `ControlSocket ... already exists, disabling multiplexing`
        and opens an unmultiplexed connection -- and an unmultiplexed `-tt`
        session is the one that used to deliver `\\r\\n` line endings where the
        multiplexed one delivered `\\n`. Same host, same command, a different
        byte stream, decided by a leftover file. That is the defect this whole
        change exists to remove, so it gets a test that creates the condition on
        purpose rather than waiting to meet it.
        """
        self.clear_sockets()
        planted = []
        try:
            for target, _ in self.prepared:
                path = control_path_for(target)
                if path:
                    with open(path, "w") as handle:
                        handle.write("not a socket")
                    planted.append(path)
            for target, arch in self.prepared:
                with self.subTest(host=target, arch=arch):
                    self.assert_matches_oracle(target, "unmultiplexed")
        finally:
            for path in planted:
                if os.path.exists(path):
                    os.remove(path)

    def test_04_the_three_transports_agree_byte_for_byte(self):
        """The three above, compared against each other rather than the oracle.

        The oracle comparisons could all three be wrong in the same way and
        still agree with it. This asks the question directly: does the transport
        change what arrives?
        """
        for target, arch in self.prepared:
            with self.subTest(host=target, arch=arch):
                self.clear_sockets()
                cold = exec_wire.output(self.run_exec(target, ORACLE_CMD))
                warm = exec_wire.output(self.run_exec(target, ORACLE_CMD))
                self.clear_sockets()
                path = control_path_for(target)
                with open(path, "w") as handle:
                    handle.write("not a socket")
                try:
                    unmuxed = exec_wire.output(self.run_exec(target, ORACLE_CMD))
                finally:
                    if os.path.exists(path):
                        os.remove(path)
                self.assertEqual(cold, warm, f"{target}: cold and warm sockets differ")
                self.assertEqual(
                    cold, unmuxed, f"{target}: multiplexing changed the bytes"
                )

    def test_05_an_alias_and_a_terminal_reach_the_command(self):
        """What the pty buys, asserted rather than assumed.

        `isatty(1)` is what `apt` and `docker` read to decide whether to use
        colour and a progress display, and the interactive login shell is what
        makes an alias like `ud` resolve -- which is what most people actually
        put in `upgrade_cmd`.
        """
        for target, arch in self.prepared:
            with self.subTest(host=target, arch=arch):
                frames = self.run_exec(target, "test -t 1 && echo TTY_YES || echo TTY_NO")
                self.assertIn(b"TTY_YES", exec_wire.output(frames))

    def test_06_the_window_size_reaches_the_command(self):
        """A pty sized to the panel, so `apt` does not wrap to 80 columns
        inside a 200-column window."""
        for target, arch in self.prepared:
            with self.subTest(host=target, arch=arch):
                frames = self.run_exec(target, "stty size", cols=203, rows=51)
                self.assertIn(b"51 203", exec_wire.output(frames))

    def test_07_the_log_is_the_command_and_nothing_else(self):
        """A login shell is not quiet at either end: `zsh -l -i` emits an
        `OSC 111` on the way in and another on the way out, both observed on a
        live host. Neither belongs in an operator's upgrade log."""
        for target, arch in self.prepared:
            with self.subTest(host=target, arch=arch):
                frames = self.run_exec(target, "echo ONLY_THIS")
                self.assertEqual(exec_wire.output(frames), b"ONLY_THIS\r\n")

    def test_08_a_failing_command_reports_its_own_code(self):
        for target, arch in self.prepared:
            with self.subTest(host=target, arch=arch):
                frames = self.run_exec(target, "exit 42")
                self.assertEqual(exec_wire.exit_code(frames), 42)

    def test_09_every_run_ends_in_an_exit_frame(self):
        """The guarantee the panel's state machine rests on. A run that cannot
        say it finished pins its panel in STARTED for the session."""
        for target, arch in self.prepared:
            for command in ["true", "exit 7", "no-such-command-anywhere", "echo x >&2"]:
                with self.subTest(host=target, command=command):
                    frames = self.run_exec(target, command)
                    self.assertIsNotNone(
                        exec_wire.exit_code(frames), f"{command!r} never reported finishing"
                    )
                    self.assertEqual(frames[-1][0], "Exit")


if __name__ == "__main__":
    unittest.main(verbosity=2)
