#!/usr/bin/env python3
"""One gate run at a time, per clone.

Two full suites at once contend the shared cargo target dir and the CPU,
and timing-sensitive suites flake under it -- an e2e upgrade going quiet
for 5s, a coverage export timing out. The failures look like product
defects for a while, and each one has now cost a debugging session.

So the pre-commit hook, the pre-push hook (via local-ci.py) and local-ci.py
itself all take this lock first. Blocking, not fail-fast: the second run's
verdict is still needed, it just waits its turn.

Usage:
    python3 tools/gate_lock.py acquire <owner-pid>   # blocks; heartbeat
    python3 tools/gate_lock.py release <owner-pid>   # only removes own lock

The owner pid is explicit because the helper is short-lived: recording the
helper's own pid would mark every lock stale the instant it exits (and
release could then never match). Callers pass their own stable pid -- $$
in shell, os.getpid() in python -- so pre-push execing local-ci under one
pid is naturally re-entrant.

The lock is a directory (mkdir is atomic) holding the holder's pid, under
.git so it is per clone. A holder that died without releasing (kill -9)
leaves a stale directory; the next acquirer reaps it when the pid is gone.
PID reuse could theoretically reap a live lock -- the consequence is two
concurrent runs, which is exactly today without the lock, so the failure
mode is the status quo, not something worse.

Escape hatch: GATE_LOCK=0 skips both commands (CI debuggers, emergencies).
This is deliberately loud about doing nothing.
"""

from __future__ import annotations

import os
import subprocess
import sys
import time

HEARTBEAT_EVERY = 60


def repo_root() -> str:
    r = subprocess.run(
        ["git", "rev-parse", "--show-toplevel"],
        capture_output=True,
        text=True,
        check=False,
    )
    if r.returncode != 0 or not r.stdout.strip():
        print("gate_lock: not in a git repo", file=sys.stderr)
        sys.exit(2)
    return r.stdout.strip()


def lockdir() -> str:
    return os.path.join(repo_root(), ".git", "gate.lock")


def holder_alive(pid: int) -> bool:
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    except PermissionError:
        return True
    return True


def read_holder(path: str) -> int | None:
    try:
        with open(os.path.join(path, "pid"), encoding="utf-8") as f:
            return int(f.read().strip().split()[0])
    except (OSError, ValueError, IndexError):
        return None


def acquire(owner: int) -> None:
    path = lockdir()
    waited = 0
    while True:
        try:
            os.mkdir(path)
        except FileExistsError:
            holder = read_holder(path)
            if holder == owner:
                return  # re-entrant: pre-push execs local-ci under one pid
            if holder is not None and holder_alive(holder):
                if waited == 0 or waited % HEARTBEAT_EVERY == 0:
                    print(
                        f"gate lock held by pid {holder}, waiting…",
                        file=sys.stderr,
                        flush=True,
                    )
                time.sleep(5)
                waited += 5
                continue
            # Stale: holder gone (or unreadable) without releasing.
            try:
                if holder is not None:
                    os.remove(os.path.join(path, "pid"))
                os.rmdir(path)
            except OSError:
                pass
            continue
        with open(os.path.join(path, "pid"), "w", encoding="utf-8") as f:
            f.write(str(owner))
        return


def release(owner: int) -> None:
    # Only ever remove our own lock. After PID reuse or a confused manual
    # run, rmdir-ing someone else's would serialize nothing and lie about it.
    path = lockdir()
    if read_holder(path) == owner:
        try:
            os.remove(os.path.join(path, "pid"))
            os.rmdir(path)
        except OSError:
            pass


def main(argv: list[str]) -> int:
    if os.environ.get("GATE_LOCK") == "0":
        print("gate_lock: GATE_LOCK=0, skipping (escape hatch)", file=sys.stderr)
        return 0
    if len(argv) != 2 or argv[0] not in ("acquire", "release"):
        print("usage: gate_lock.py [acquire|release] <owner-pid>", file=sys.stderr)
        return 2
    try:
        owner = int(argv[1])
    except ValueError:
        print("usage: gate_lock.py [acquire|release] <owner-pid>", file=sys.stderr)
        return 2
    if argv[0] == "acquire":
        acquire(owner)
    else:
        release(owner)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
