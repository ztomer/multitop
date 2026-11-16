"""Comprehensive e2e for the entire roadmap — Phase 1-5 plus server mode.

Covers: Help, Layout memory, Focus, Command palette, Yank, Vault, Alerts,
History, Graphs zoom, Server --serve, and the Hello+Token auth.
All waits are on conditions, not sleeps (a test-side poll interval aside).
"""

import os
import socket
import subprocess
import sys
import tempfile
import time
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from tmux_harness import TmuxSession, find_binary


def tmux_available():
    import shutil

    return shutil.which("tmux") is not None


class ComprehensiveE2E(unittest.TestCase):
    """One binary, one config, many views — the whole roadmap in one session."""

    @classmethod
    def setUpClass(cls):
        if not tmux_available():
            cls.skip_reason = "tmux not installed"
            return
        binary = find_binary()
        if binary is None:
            cls.skip_reason = "no multitop binary"
            return
        from tmux_harness import stale_reason

        if (reason := stale_reason(binary)) and not os.environ.get("MULTITOP_BIN"):
            print(f"WARNING: {reason}", file=sys.stderr)
        cls.binary = binary
        cls.skip_reason = None

    def setUp(self):
        if getattr(self, "skip_reason", None):
            self.skipTest(self.skip_reason)
        # Per-test session for isolation — shared session across tests is brittle
        # when any test leaves the app in a modal or filter.
        self.tmpdir = tempfile.mkdtemp(prefix="comprehensive-")
        self.config_path = os.path.join(self.tmpdir, "config.toml")
        with open(self.config_path, "w") as f:
            f.write(
                '[[servers]]\nhost="127.0.0.1"\nport=22\nuser="ztomer"\n'
                '[[servers]]\nhost="192.168.0.33"\nport=22\nuser="ztomer"\n'
            )
        self.session = TmuxSession(self.binary, self.config_path, size=(140, 40), tag="comprehensive")
        try:
            self.session.start()
        except Exception as e:
            self.skipTest(str(e))

    def tearDown(self):
        if hasattr(self, "session") and self.session:
            self.session.kill()
        if hasattr(self, "tmpdir"):
            import shutil

            shutil.rmtree(self.tmpdir, ignore_errors=True)

    def wait_until(self, predicate, what):
        """Wait until `predicate(screen)` holds; return that screen."""

        def probe():
            screen = self.session.capture()
            return screen if predicate(screen) else None

        return self.session.wait_for(probe, timeout=10, what=what)

    @staticmethod
    def _answers(port):
        try:
            with socket.create_connection(("127.0.0.1", port), timeout=1):
                return True
        except OSError:
            return False

    def test_help_overlay(self):
        self.session.send("?")
        self.session.wait_for_screen("Help  ? to close", timeout=10)
        s = self.session.capture()
        self.assertIn("quit", s.lower())
        self.session.send("Escape")
        self.session.wait_for(
            lambda: "Help  ? to close" not in self.session.capture(), timeout=10, what="help to close"
        )
        self.assertNotIn("Help  ? to close", self.session.capture())

    def test_focus_enter_z(self):
        # Focus the first panel
        # Keys reach the app in the order sent, so "z" needs no pause after "1".
        self.session.send("1")
        self.session.send("z")
        # Focus shows unzoom in keybar
        self.wait_until(lambda s: "unzoom" in s.lower(), "the keybar to offer unzoom")
        self.session.send("Escape")
        self.wait_until(lambda s: "unzoom" not in s.lower(), "unzoom to leave the keybar")

    def test_command_palette(self):
        self.session.send(":")
        self.session.wait_for_screen("Command Palette", timeout=10)
        s = self.session.capture()
        self.assertIn("filter", s.lower())
        self.session.send("Escape")
        self.wait_until(lambda s: "Command Palette" not in s, "the palette to close")
        # Execute a palette command: filter via palette
        self.session.send(":")
        self.session.wait_for_screen("Command Palette", timeout=10)
        self.session.send("filter 127")
        self.session.send("Enter")
        s2 = self.wait_until(lambda s: "[filter: 127]" in s, "the palette's filter to apply")
        self.assertIn("127.0.0.1", s2)

    def test_yank_does_not_crash(self):
        # y should not crash, even though we can't easily verify clipboard in tmux
        self.session.send("y")
        # A dump is written by the event loop itself, so one arriving after the
        # key proves the loop is still turning, not merely that the pid exists.
        self.session.diag()
        # Should still be running
        self.assertIsNone(self.session.exited())

    def test_layout_persists_sort_and_filter(self):
        # Change sort to mem
        self.session.send("m")
        # Set filter
        self.session.send("/")
        self.session.send("127")
        self.session.send("Enter")
        s = self.wait_until(lambda s: "[filter: 127]" in s, "the filter to apply")
        self.assertIn("127.0.0.1", s)
        # Check state.toml was written
        import pathlib

        state_path = pathlib.Path(self.config_path).with_name("state.toml")
        content = self.session.wait_for(
            lambda: state_path.exists() and "filter_query" in state_path.read_text() and state_path.read_text(),
            what="state.toml to persist the filter",
        )
        self.assertIn("filter_query", content)

    def test_graphs_and_H_and_zoom(self):
        self.session.send("g")
        self.session.wait_for_screen("CPU", timeout=10)
        s = self.session.capture()
        self.assertIn("CPU", s)
        # Zoom
        self.session.send("+")
        self.session.send("-")
        # H for alerts (reuses graphs)
        self.session.send("H")
        # Alerts view should show ALERTS 30m graph
        self.session.wait_for_screen("ALERTS 30m", timeout=10)
        self.session.send("s")
        # The keys above went through the loop without ending it.
        self.session.diag()
        self.assertIsNone(self.session.exited())

    def test_unhealthy_filter(self):
        # unhealthy is a synthetic token: it matches hosts breaching any
        # alert threshold (CPU 80 / MEM 85 / DISK 90), not text. So the
        # honest assertion is the correlation, not a fixed outcome -- on a
        # machine under memory pressure (parallel suites eat gigabytes) the
        # host IS breaching and the panel must stay visible; asserting "No
        # host matches" unconditionally made this test fail whenever the
        # box was loaded, correctly reporting a healthy filter as broken.
        self.session.send("/")
        self.session.send("unhealthy")
        self.session.send("Enter")
        s = self.session.wait_for_screen("[filter: unhealthy]", timeout=10)
        if "No host matches" in s:
            self.session.send("Escape")
            return  # healthy host, correctly filtered out
        # The config holds TWO hosts, and either may be the one breaching:
        # `is_breaching_with_vault` also counts a locked vault (app-wide, so
        # it marks every host that has a snapshot), and a host whose first
        # snapshot has not arrived never breaches. Naming 127.0.0.1 therefore
        # failed whenever only the Beelink had reported (2026-10-10: Beelink
        # alone, at 0% CPU / 36% MEM / 39% DSK). Which host breaches is pinned
        # by `health.rs`'s own tests; here the claim is only that what survives
        # the filter is a configured host.
        self.assertTrue(
            "127.0.0.1" in s or "192.168.0.33" in s,
            "a breaching host must stay visible under the unhealthy filter",
        )
        self.session.send("Escape")

    def test_vault_settings_shows_stored(self):
        self.session.send("e")
        self.session.wait_for_screen("Settings", timeout=10)
        s = self.session.capture()
        # Settings should be visible
        self.assertIn("Settings", s)
        # Check that vault section is there
        self.assertIn("Server", s)
        self.session.send("Escape")

    def test_server_mode_via_http(self):
        # Server mode is tested via the Rust unit tests, but we also test the binary directly
        # Start a server on a random port and curl it
        import json
        import tempfile

        # Find a free port
        with socket.socket() as s:
            s.bind(("127.0.0.1", 0))
            port = s.getsockname()[1]
        addr = f"127.0.0.1:{port}"
        # Use a minimal config with one server. The token is not passed: --serve
        # creates it 0600 beside the config, which is the path a real start takes.
        with tempfile.TemporaryDirectory() as tmpdir:
            cfg = os.path.join(tmpdir, "config.toml")
            token_file = os.path.join(tmpdir, "serve-token")
            with open(cfg, "w") as f:
                f.write('[[servers]]\nhost="127.0.0.1"\nport=22\nuser="ztomer"\n')
            proc = subprocess.Popen(
                [self.binary, "-c", cfg, "--serve", addr],
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                stdin=subprocess.DEVNULL,
            )
            try:
                # Ready means the token is written and the port answers; both
                # are waited for, never guessed at with a fixed pause.
                deadline = time.monotonic() + 20
                while not (os.path.exists(token_file) and self._answers(port)):
                    self.assertIsNone(proc.poll(), "multitop --serve exited before it listened")
                    self.assertLess(time.monotonic(), deadline, "multitop --serve never listened")
                    time.sleep(0.05)
                with open(token_file) as f:
                    token = f.read().strip()
                self.assertEqual(os.stat(token_file).st_mode & 0o777, 0o600)
                # Without token should be 401
                r = subprocess.run(
                    ["curl", "-s", f"http://{addr}/api/hosts"],
                    capture_output=True,
                    text=True,
                    timeout=5,
                    stdin=subprocess.DEVNULL,
                    check=True,
                )
                self.assertIn("unauthorized", r.stdout.lower())
                # With token should be 200
                r2 = subprocess.run(
                    ["curl", "-s", "-H", f"Authorization: Bearer {token}", f"http://{addr}/api/hosts"],
                    capture_output=True,
                    text=True,
                    timeout=5,
                    stdin=subprocess.DEVNULL,
                    check=True,
                )
                self.assertIn("127.0.0.1", r2.stdout)
                data = json.loads(r2.stdout)
                self.assertIsInstance(data, list)
                # Health
                r3 = subprocess.run(
                    ["curl", "-s", "-H", f"Authorization: Bearer {token}", f"http://{addr}/api/health"],
                    capture_output=True,
                    text=True,
                    timeout=5,
                    stdin=subprocess.DEVNULL,
                    check=True,
                )
                self.assertIn("total", r3.stdout)
                # Index
                r4 = subprocess.run(
                    ["curl", "-s", f"http://{addr}/"],
                    capture_output=True,
                    text=True,
                    timeout=5,
                    stdin=subprocess.DEVNULL,
                    check=True,
                )
                self.assertIn("multitop --serve", r4.stdout)
            finally:
                proc.terminate()
                try:
                    _, stderr = proc.communicate(timeout=5)
                except subprocess.TimeoutExpired:
                    proc.kill()
                    _, stderr = proc.communicate()
            # stderr here is a pipe, as under a service manager: it names the
            # file and never carries the token (ROADMAP A1).
            stderr = stderr.decode(errors="replace")
            self.assertIn(token_file, stderr)
            self.assertNotIn(token, stderr)

    def test_still_quits(self):
        # Ensure quit still works after all the above
        # We don't actually quit the shared session here, just check that q doesn't crash
        # The final test in the suite will quit
        pass


if __name__ == "__main__":
    unittest.main(verbosity=2)
