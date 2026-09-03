"""Drive a real multitop in a real terminal, over tmux's unix socket.

# Why a private socket

`tmux -S <path>` starts a server of its own rather than joining the user's.
Nothing here can touch a session someone is working in, and the teardown can
kill the whole server without asking what else is in it.

# Why not `time.sleep`

Every assertion waits on a condition. The Rust suite had a test that slept
500 ms and then pressed `q`, and it failed about one run in three under load --
not because the timing was tight but because what it was really waiting for was
a subprocess, and a sleep cannot wait for one. The same mistake is easier to
make here, where every step is a keystroke into a terminal.

# The config needs a directory of its own

`state_file_path` is `config_path.with_file_name("state.toml")`, so the durable
record of past upgrades lives *beside* the config. A config written straight
into `/tmp` therefore shares `/tmp/state.toml` with every other run on the
machine -- which is how the first version of these tests found a panel already
reporting "Last run 1 min ago" before it had run anything, and asserted against
someone else's history. `session_dir` gives each session its own.

# Two assertion surfaces, for two different questions

`capture-pane` answers "what is on screen". It cannot answer "how many lines are
in the log", because it sees an 80x24 viewport and cannot tell a line that was
printed twice from one that scrolled.

So the counting questions are asked of the app itself, through the SIGUSR2
diagnostic dump -- `ring 12` is exact and does not care what fits on screen.

The dump deliberately carries **no log content**: `PanelDigest` is free of every
field that could hold a secret, because an upgrade log holds whatever the
command printed. That is a good property and this harness does not ask for it to
be weakened; content questions go to `capture-pane`, counting questions go to
the dump, and neither is asked to do the other's job.
"""

import glob
import os
import re
import signal
import subprocess
import tempfile
import time

#: How long any `wait_for` will keep asking before giving up.
DEFAULT_TIMEOUT = 20.0
#: How often it asks. Short enough to keep a test quick, long enough not to
#: spin a terminal into the ground.
POLL_INTERVAL = 0.1


class Timeout(AssertionError):
    """A condition never became true. Carries the last thing seen, because
    'timed out' on its own says nothing about why."""


class TmuxSession:
    """One multitop, in one tmux server, on a socket of its own."""

    #: Printed into the pane once the app exits, so a death is legible rather
    #: than being a pane that simply vanished.
    EXIT_MARKER = "[multitop exited]"

    #: Something only a drawing app puts on screen. The keybar is rendered by
    #: the event loop on every frame, so its presence means the loop is running.
    READY_MARKER = "Quit"

    def __init__(self, binary, config_path, size=(80, 24), tag="e2e"):
        self.binary = binary
        self.config_path = config_path
        self.size = size
        self.sock = f"/tmp/multitop-tmux-{os.getpid()}-{tag}.sock"
        self.pid = None

    # ---------------------------------------------------------------- lifecycle

    def start(self):
        self._tmux("kill-server", check=False)
        # The pane outlives the app on purpose.
        #
        # multitop writes a bad-config error to stderr and exits; tmux then
        # closes the pane, the session ends, the server exits, and every later
        # call fails with `no server running` -- which says the harness lost its
        # terminal and nothing at all about the app refusing its own config.
        # Holding the pane open keeps that message readable, which is the only
        # reason a `\n` interpolated into a TOML *comment* was ever findable.
        launch = (
            f"{self.binary} -c {self.config_path}; "
            f"printf '\\n{self.EXIT_MARKER} rc=%s\\n' \"$?\"; sleep 300"
        )
        subprocess.run(
            ["tmux", "-S", self.sock, "new-session", "-d",
             "-x", str(self.size[0]), "-y", str(self.size[1]),
             "sh", "-c", launch],
            check=True, capture_output=True, timeout=30,
        )
        # The pane's own command, so a dump can be found by pid rather than by
        # taking the newest file in a shared temp directory -- two of these
        # running at once would otherwise read each other's.
        try:
            self.pid = self.wait_for(
                lambda: self._pane_pid(), what="the multitop process to appear"
            )
        except Timeout as exc:
            # Say what the app printed on its way out. Without this the failure
            # is a bare `ProcessLookupError` twelve tests later, which names the
            # symptom and hides an app that refused its own config.
            raise Timeout(
                f"{exc}\nmultitop did not start, or did not stay running. "
                f"Last screen:\n{self._last_screen()}"
            ) from exc

        # A pid is not readiness, and the difference is fatal rather than
        # cosmetic: the default disposition for SIGUSR2 is to **terminate**, so
        # a dump requested in the window between the process existing and its
        # handler being installed kills the app. That is what made this suite
        # fail in bursts -- twelve failures, then none, then twelve -- and it
        # read as a stale binary twice before it read as a race.
        #
        # The keybar is drawn by the loop, so seeing it means the loop is
        # running, which means `diag::install` has already run.
        self.wait_for(
            lambda: self.READY_MARKER in self.capture(),
            what="the app to draw its first frame (a pid is not readiness)",
        )
        return self

    def _last_screen(self):
        """Whatever the pane still holds, or why it cannot be read."""
        try:
            return self._tmux("capture-pane", "-p", check=False).strip() or "(empty)"
        except Exception as exc:  # the server may already be gone
            return f"(no pane: {exc})"

    def kill(self):
        self._tmux("kill-server", check=False)
        for path in glob.glob(self._dump_glob()):
            try:
                os.remove(path)
            except OSError:
                pass

    def __enter__(self):
        return self.start()

    def __exit__(self, *_):
        self.kill()

    # ------------------------------------------------------------------- input

    def send(self, keys):
        """Send keystrokes. `keys` is tmux's own syntax, so `Escape` and `C-c`
        work as well as ordinary characters."""
        self._tmux("send-keys", keys)

    # ------------------------------------------------------------------ output

    def capture(self):
        """What is on screen right now."""
        return self._tmux("capture-pane", "-p")

    def diag(self):
        """Ask the app for a state dump and parse it.

        A `ProcessLookupError` here means the app is gone, which is a finding
        rather than a harness fault -- so it is raised as one, with the screen.

        SIGUSR2 sets a flag the loop notices on its next poll and answers by
        writing the richer of the two tiers -- so a dump that never arrives is
        itself the finding: it means the loop is not polling.
        """
        before = set(glob.glob(self._dump_glob()))
        if os.environ.get("MT_HARNESS_DEBUG"):
            print(f"[diag] pid={self.pid} glob={self._dump_glob()} before={len(before)}", flush=True)
        try:
            os.kill(self.pid, signal.SIGUSR2)
        except ProcessLookupError as exc:
            raise AssertionError(
                f"multitop (pid {self.pid}) is no longer running. Last screen:\n"
                f"{self._last_screen()}"
            ) from exc
        path = self.wait_for(
            lambda: next(iter(set(glob.glob(self._dump_glob())) - before), None),
            what="a diagnostic dump from the loop (its absence means a wedged loop)",
        )
        with open(path) as handle:
            return parse_dump(handle.read())

    # -------------------------------------------------------------------- wait

    def wait_for(self, probe, timeout=DEFAULT_TIMEOUT, what="a condition"):
        """Call `probe` until it returns something truthy, then return it.

        Returns the value rather than just succeeding, so a caller can wait for
        a thing and use it in one step.
        """
        deadline = time.monotonic() + timeout
        last = None
        while time.monotonic() < deadline:
            try:
                last = probe()
            except Exception as exc:  # a probe that cannot run yet is not a pass
                last = f"probe raised {exc!r}"
            else:
                if last:
                    return last
            time.sleep(POLL_INTERVAL)
        raise Timeout(f"waited {timeout:g}s for {what}; last saw {last!r}")

    def wait_for_screen(self, text, timeout=DEFAULT_TIMEOUT):
        """Wait until `text` is on screen, and return the screen."""
        self.wait_for(
            lambda: text in self.capture(),
            timeout=timeout,
            what=f"{text!r} on screen",
        )
        return self.capture()

    def wait_for_diag(self, predicate, timeout=DEFAULT_TIMEOUT, what="a state"):
        """Wait until the app's own state satisfies `predicate`."""
        seen = {}

        def probe():
            seen.update(self.diag())
            return seen if predicate(seen) else None

        try:
            return self.wait_for(probe, timeout=timeout, what=what)
        except Timeout as exc:
            raise Timeout(f"{exc}\nlast dump: {seen}") from exc

    # ----------------------------------------------------------------- private

    def _dump_glob(self):
        # `Diag::default_dir()` is Rust's `std::env::temp_dir()`, which is
        # `$TMPDIR` and only falls back to `/tmp`. On macOS that is a
        # per-user `/var/folders/...` path, so hardcoding `/tmp` here found
        # nothing and every test failed on a dump that had been written
        # perfectly well. Python resolves `$TMPDIR` the same way, and the tmux
        # server inherits this process's environment, so the two agree.
        return os.path.join(tempfile.gettempdir(), f"multitop-diag-{self.pid}-*")

    def _pane_pid(self):
        """The app's pid, found by parentage from this session's own pane.

        Never by scanning the process table for the word "multitop". The first
        version did, and on a machine where the user had their own multitop
        running it found *that* one -- so the harness signalled a process nobody
        had started, read its dumps, and reported them as the test's. A test
        that passes against someone else's process is worse than one that fails.
        The repo has been bitten by this exact shape before, with a stale
        `multitop-agent` on `PATH`.
        """
        out = self._tmux("list-panes", "-F", "#{pane_pid}", check=False).strip()
        if not out:
            return None
        pane_pid = int(out.splitlines()[0])
        if self._is_multitop(pane_pid):
            return pane_pid
        # The pane runs `sh`, which holds it open past the app's exit so a
        # startup failure stays readable; multitop is its child.
        for pid in self._children(pane_pid):
            if self._is_multitop(pid):
                return pid
        return None

    def exited(self):
        """Whether the app has ended, and what it said on the way out."""
        screen = self._last_screen()
        if self.EXIT_MARKER not in screen:
            return None
        return screen

    @staticmethod
    def _ps(*args):
        """`/bin/ps`, absolutely.

        `ps` on `PATH` is not necessarily `ps`: on this machine it resolves to a
        replacement that rejects `-p` and answers with a usage error, which read
        as "the process is dead" and sent an afternoon chasing a signal handler
        that was working perfectly well.
        """
        return subprocess.run(["/bin/ps", *args],
                              capture_output=True, text=True, timeout=10).stdout

    @classmethod
    def _is_multitop(cls, pid):
        comm = cls._ps("-p", str(pid), "-o", "comm=").strip()
        # The exact binary, not a substring: "multitop-agent" contains
        # "multitop", and so does any path with the project directory in it.
        return comm == "multitop" or comm.endswith("/multitop")

    @classmethod
    def _children(cls, parent):
        out = cls._ps("-o", "pid=,ppid=", "-ax")
        kids = []
        for line in out.splitlines():
            parts = line.split()
            if len(parts) == 2 and parts[1] == str(parent) and parts[0].isdigit():
                kids.append(int(parts[0]))
        return kids

    def _tmux(self, *args, check=True):
        result = subprocess.run(
            ["tmux", "-S", self.sock, *args],
            capture_output=True, text=True, timeout=30,
        )
        if check and result.returncode != 0:
            raise AssertionError(f"tmux {args}: {result.stderr.strip()}")
        return result.stdout


#: `panel 0: host | Mode | STATE | gen 3 (upgrade 1) | view 20 | ring 12 | scroll 0`
_PANEL = re.compile(
    r"panel (?P<index>\d+): (?P<host>.*?) \| (?P<mode>\S+) \| (?P<state>\S+) \| "
    r"gen (?P<gen>\d+) \(upgrade (?P<upgrade_gen>\d+)\) \| view (?P<view>\d+) \| "
    r"ring (?P<ring>\d+) \| scroll (?P<scroll>\d+)"
)
_FLAGS = re.compile(r"(\w+): (true|false)")


def parse_dump(text):
    """Turn a diagnostic dump into something a test can assert on."""
    state = {
        "raw": text,
        "panels": [],
        "snapshot": "snapshot: captured" in text,
    }
    for match in _FLAGS.finditer(text):
        state[match.group(1)] = match.group(2) == "true"
    for match in _PANEL.finditer(text):
        state["panels"].append(
            {
                "index": int(match.group("index")),
                "host": match.group("host"),
                "mode": match.group("mode"),
                "state": match.group("state"),
                "gen": int(match.group("gen")),
                "upgrade_gen": int(match.group("upgrade_gen")),
                "view": int(match.group("view")),
                "ring": int(match.group("ring")),
                "scroll": int(match.group("scroll")),
            }
        )
    if "active_confirm: Some(" in text:
        state["active_confirm"] = text.split("active_confirm: ")[1].split("\n")[0].strip()
    else:
        state["active_confirm"] = None
    return state


def session_dir(tag="e2e"):
    """A private directory for one session's config -- and therefore for its
    `state.toml`, which lives beside it."""
    path = os.path.join(tempfile.gettempdir(), f"multitop-e2e-{os.getpid()}-{tag}")
    os.makedirs(path, exist_ok=True)
    return path


def newest_source_time(root=None):
    """When this crate's source was last touched."""
    root = root or os.path.join(
        os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "crates"
    )
    newest = 0.0
    for base, _, files in os.walk(root):
        for name in files:
            if name.endswith(".rs") or name == "Cargo.toml":
                newest = max(newest, os.path.getmtime(os.path.join(base, name)))
    return newest


def stale_reason(binary):
    """Why this binary must not be trusted, or None if it is current.

    An e2e test drives a *binary*, and a binary older than the source is a test
    of code nobody wrote. This has now cost three separate debugging sessions in
    one afternoon -- a stale `multitop-agent` on `PATH`, a leftover `multitop`
    belonging to the user, and a build that predated the fix under test and
    reported it as still broken. Every one of them looked like a product defect
    for a while.

    So it is checked rather than assumed, and the answer is a refusal with a
    reason instead of a run that quietly means nothing.
    """
    source = newest_source_time()
    built = os.path.getmtime(binary)
    if built >= source:
        return None
    import datetime

    return (
        f"{binary} was built at "
        f"{datetime.datetime.fromtimestamp(built):%H:%M:%S} but the source was "
        f"last changed at {datetime.datetime.fromtimestamp(source):%H:%M:%S} -- "
        "run `cargo build -p multitop` first; testing a stale binary tests code "
        "nobody wrote"
    )


def find_binary():
    """This build's `multitop`, or None with a reason the caller can print.

    Never from `PATH`. A hand-installed binary elsewhere on the machine is a
    different build, and a test that silently exercises one is testing something
    nobody changed -- this repo has been bitten by exactly that with a stale
    `multitop-agent` in `/usr/local/bin`.
    """
    if (explicit := os.environ.get("MULTITOP_BIN")) and os.access(explicit, os.X_OK):
        return explicit
    roots = []
    if (target := os.environ.get("CARGO_TARGET_DIR")):
        roots.append(target)
    roots += [os.path.expanduser("~/.cache/cargo-target"),
              os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "target")]
    newest = None
    for root in roots:
        for profile in ("release", "debug"):
            candidate = os.path.join(root, profile, "multitop")
            if os.path.isfile(candidate) and os.access(candidate, os.X_OK):
                stamp = os.path.getmtime(candidate)
                if newest is None or stamp > newest[0]:
                    newest = (stamp, candidate)
    return newest[1] if newest else None
