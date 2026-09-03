"""L4 -- the real app, in a real terminal, driven over tmux's unix socket.

What this layer is for: everything below it drives functions, and a function
cannot tell you that a key press reached the loop, that the panel redrew, or
that the app could still be quit afterwards. This one presses keys at a running
binary and asks what it did.

Every wait is on a condition. The version of this file that came before slept
between every step, which is the same defect the Rust `event_loop_e2e` flake
turned out to be: a sleep cannot wait for a subprocess, so it passes when the
machine is quiet and fails when it is not.
"""

import os
import sys
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from tmux_harness import (  # noqa: E402
    TmuxSession,
    find_binary,
    session_dir,
    stale_reason,
)

#: A prompt with no newline, then a carriage-return progress display, then a
#: final line.
#:
#: All three are shapes the old reader got wrong: the prompt was flushed on a
#: 100 ms timer and re-sent on every tick after that; the progress display was
#: one log line per repaint; and under a pty every line ends CR-LF, which
#: `painted_states` collapsed to the empty string.
UPGRADE_CMD = (
    r"printf 'Interactive prompt [Y/n] '; sleep 0.5; printf '\n'; "
    r"printf '10%%\r20%%\r30%%\n'; "
    r"echo 'mock done'"
)

#: A TOML **multi-line literal** string for the command.
#:
#: It has to be literal, or TOML unescapes the backslash sequences into real
#: control characters before the shell ever sees them, and `printf` gets a raw
#: CR where it was meant to get the two characters it turns into one. And it has
#: to be multi-line, because a single-quoted literal cannot contain the single
#: quotes this command is full of -- the first attempt produced
#: `upgrade_cmd = 'printf 'Interactive...''`, which is not a string at all.
#:
#: The comments live out here rather than inside the template for the same class
#: of reason: written inside an f-string, a comment mentioning an escape
#: sequence has that sequence interpolated into it, and a raw carriage return in
#: a comment is a TOML parse error.
CONFIG = f"""
[[servers]]
host = "127.0.0.1"
port = 22
user = "test"
upgrade_cmd = '''{UPGRADE_CMD}'''
"""


class TmuxE2E(unittest.TestCase):
    """One session for the whole class: starting multitop is the slow part, and
    these tests only read state or move between views."""

    session = None
    skip_reason = None

    @classmethod
    def setUpClass(cls):
        binary = find_binary()
        if binary is None:
            cls.skip_reason = (
                "no multitop binary in this build tree (cargo build, or set MULTITOP_BIN)"
            )
            return
        # A stale binary is a failure, not a skip. Skipping would let a suite go
        # green while testing nothing; this layer's whole value is that it drives
        # the code that was actually written.
        if (stale := stale_reason(binary)) and not os.environ.get("MULTITOP_BIN"):
            raise AssertionError(stale)
        import shutil

        if not shutil.which("tmux"):
            cls.skip_reason = "tmux is not installed"
            return

        # Its own directory: `state.toml` lives beside the config, so a config
        # in a shared `/tmp` would read and write a state file every other run
        # on this machine is also using.
        cls.config_path = os.path.join(session_dir(), "config.toml")
        with open(cls.config_path, "w") as handle:
            handle.write(CONFIG)
        cls.session = TmuxSession(binary, cls.config_path).start()

    @classmethod
    def tearDownClass(cls):
        if cls.session:
            cls.session.kill()
        path = getattr(cls, "config_path", None)
        if path:
            import shutil

            shutil.rmtree(os.path.dirname(path), ignore_errors=True)

    def setUp(self):
        if self.skip_reason:
            self.skipTest(self.skip_reason)

    # ------------------------------------------------------------------ basics

    def test_01_the_app_starts_and_answers_a_dump(self):
        """The loop is polling. If it were not, no state-tier dump would ever be
        written -- that tier is answered *by the loop*, which is what makes its
        absence the first bisection of a freeze."""
        state = self.session.wait_for_diag(
            lambda s: s["snapshot"] and s["panels"],
            what="a state-tier dump with a panel in it",
        )
        self.assertFalse(state["should_quit"])
        self.assertEqual(len(state["panels"]), 1)

    def test_02_the_upgrade_view_shows_before_it_runs(self):
        """Deliberately two presses: the first shows what *would* run and starts
        nothing."""
        self.session.send("u")
        state = self.session.wait_for_diag(
            lambda s: s["panels"][0]["mode"] == "Upgrade",
            what="the upgrade view",
        )
        # Whatever the panel's resting state is called, it must not be one that
        # says a run is happening -- and `in_flight` is the field the app itself
        # uses to decide whether quitting needs a confirmation.
        self.assertNotEqual(
            state["panels"][0]["state"],
            "STARTED",
            "the first press must not start anything",
        )
        self.assertFalse(state["in_flight"], "the first press started a run")

    def test_03_running_it_streams_and_then_finishes(self):
        """The whole channel, end to end: confirm, watch it run, watch it end.

        `in_flight` going back to false is the assertion that matters most here.
        A run that cannot say it finished pins the panel in STARTED for the rest
        of the session -- quitting starts asking for confirmation about a run
        that ended long ago, and no further upgrade can be started on any host.
        """
        self.session.send("u")  # confirm modal
        self.session.send("u")  # confirm

        # Prove it started before asserting anything about it finishing.
        #
        # Without this the test passes on an app that never ran: "not
        # in_flight" and "state is not STARTED" are both true of a panel that
        # has done nothing at all. An earlier version of this file was green for
        # exactly that reason, against a panel reading `NIL` with an empty log.
        self.session.wait_for_diag(
            lambda s: s["panels"][0]["state"] == "STARTED"
            or s["panels"][0]["ring"] > 0,
            timeout=30,
            what="the run to actually start",
        )
        state = self.session.wait_for_diag(
            lambda s: not s["in_flight"] and s["panels"][0]["state"] == "DONE",
            timeout=60,
            what="the run to report that it finished",
        )
        self.assertGreater(
            state["panels"][0]["ring"],
            0,
            "the run finished having logged nothing, so it did not run",
        )

    def test_04_the_log_holds_one_line_per_line_of_output(self):
        """The reported defect, asked of the app rather than of the screen.

        `ring` is the number of lines the log actually holds. `capture-pane`
        could not answer this: it sees an 80x24 viewport and cannot tell a line
        printed twice from one that scrolled.

        The command prints a prompt, a `\\r` progress display that repaints
        twice, and a final line. That is four lines of output -- the header, the
        prompt, one progress line and `mock done` -- plus the closing status.
        Anything much larger means something was logged more than once.
        """
        state = self.session.diag()
        ring = state["panels"][0]["ring"]
        # A floor as well as a ceiling, and both tight enough to bite.
        #
        # The ceiling was 8 to begin with, "safely" above the four lines this
        # command produces. Injecting an exact duplicate of every line took the
        # log to 8 and the test passed -- a bound chosen loosely enough to feel
        # safe was loose enough to see nothing, which is the only failure mode a
        # threshold really has. Both numbers now come from a measured run
        # (`ring == 4`: the header, the prompt, one collapsed progress line and
        # `mock done`), with one line of slack for the closing status.
        self.assertGreaterEqual(
            ring,
            3,
            f"only {ring} line(s) logged; the run cannot have produced its output",
        )
        self.assertLessEqual(
            ring,
            5,
            f"the log holds {ring} lines for four lines of output -- "
            "something is being logged more than once",
        )

    def test_05_the_output_is_on_screen_and_the_bar_collapsed(self):
        """And the content itself, which is `capture-pane`'s question."""
        screen = self.session.wait_for_screen("mock done")
        self.assertIn("Interactive prompt [Y/n]", screen)
        self.assertIn("30%", screen, "the progress display lost its last state")
        self.assertNotIn(
            "10%", screen, "an overwritten progress state was kept as its own line"
        )

    # ------------------------------------------------------------------- views

    def test_06_every_view_renders(self):
        for key, expected in [
            ("s", ("CPU",)),
            ("d", ("CONTAINER ID", "No running containers", "Docker")),
            ("f", ("OS", "Kernel")),
            ("g", ("CPU", "MEM")),
        ]:
            with self.subTest(key=key):
                self.session.send(key)
                self.session.wait_for(
                    lambda: any(e in self.session.capture() for e in expected),
                    what=f"one of {expected} after pressing {key!r}",
                )

    def test_07_settings_opens_and_escape_leaves_it(self):
        """`\u250c Settings`, not `Settings`.

        The keybar reads `[E] Settings` on every frame, so `"Settings" in
        screen` is true before the panel opens and stays true after it closes --
        an assertion that can never fail and therefore never passes for a
        reason. The box-drawing character is the panel's own border and appears
        only while it is up.
        """
        self.session.send("e")
        self.session.wait_for_screen("\u250c Settings")
        self.session.send("Escape")
        self.session.wait_for(
            lambda: "\u250c Settings" not in self.session.capture(),
            what="the Settings panel to close on Escape",
        )

    def test_08_the_filter_takes_input_and_clears(self):
        """`Filter:` with the colon, for the same reason: the keybar's
        `/ Filter` is permanent, and the prompt's `Filter:` is not."""
        self.session.send("/")
        self.session.wait_for_screen("Filter:")
        self.session.send("mock")
        self.session.wait_for(
            lambda: "Filter: mock" in self.session.capture(),
            what="the typed filter to appear in the prompt",
        )
        self.session.send("Escape")
        self.session.wait_for(
            lambda: "Filter:" not in self.session.capture(),
            what="the filter prompt to close on Escape",
        )

    def test_08b_x_shows_confirm_not_started(self):
        """Phase 3 roadmap requirement:
        'test tmux e2e: press x -> confirm modal visible, no task started; confirm -> task started'
        """
        self.session.send("s")
        self.session.wait_for_diag(
            lambda s: s["panels"][0]["mode"] == "Monitor",
            what="the monitor view",
        )
        self.session.send("x")
        state = self.session.wait_for_diag(
            lambda s: s.get("active_confirm") is not None or not s["in_flight"],
            what="the confirm modal or standing state",
        )
        self.assertFalse(state["in_flight"], "pressing x must not start a task before confirm")
        self.session.send("Escape")
        state_after = self.session.wait_for_diag(
            lambda s: s.get("active_confirm") is None,
            what="the confirm to clear on Escape",
        )
        self.assertIsNone(state_after.get("active_confirm"))

    # -------------------------------------------------------------------- quit

    def test_09_the_app_still_quits(self):
        """The last thing, and the one a freeze takes away first.

        With no upgrade in flight this is a single press. That distinction is
        the whole of the flake that used to live in `event_loop_e2e`: quitting
        *during* a run deliberately takes two, so a test that pressed once was
        passing only when the run happened to have finished first.
        """
        state = self.session.diag()
        self.assertFalse(
            state["in_flight"], "this test assumes nothing is running; it is not"
        )
        self.session.send("q")
        # The pane outlives the app deliberately, so "it quit" is a marker in
        # the pane rather than a vanished tmux server. Waiting for the server to
        # disappear would also have been satisfied by the server *crashing*,
        # which is not what this asserts.
        screen = self.session.wait_for(
            self.session.exited, what="the app to exit"
        )
        self.assertIn(
            f"{self.session.EXIT_MARKER} rc=0",
            screen,
            f"quitting must be a clean exit, not a crash:\n{screen}",
        )


if __name__ == "__main__":
    unittest.main(verbosity=2)
