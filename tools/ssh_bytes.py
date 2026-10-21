#!/usr/bin/env python3
"""A PATH shim that counts the bytes an `ssh` session carries.

The Ops poll opens each host's `mcp_host` with `ssh`, so what the host
sends back is exactly what the client receives. This shim stands in for
`ssh` on the PATH, passes the session through untouched, and counts what
crosses it.

The protocol is one JSON-RPC message per line, so the shim counts the
bytes of each ANSWER separately and says which request it answers - the
bytes the client receives, counted per answer.

The counts are written out as they grow, not at the end: the client kills
the session's process group when it is done with it, and a shim that kept
its tally to itself would be killed with it and lose the tally.

It is a MEASUREMENT, not a proxy: the bytes are counted on the plaintext
side of the encryption, which is the side the client reads, and the session
itself is the real `ssh` with the real arguments.

Environment:
  MEASURE_SSH_REAL  the real ssh to run (default /usr/bin/ssh)
  MEASURE_SSH_LOG   the file to append one JSON record per count to
"""
import json
import os
import subprocess
import sys
import threading
import time
import uuid

REAL = os.environ.get("MEASURE_SSH_REAL", "/usr/bin/ssh")
LOG = os.environ.get("MEASURE_SSH_LOG", "")
# How often the running tally is written out.
EVERY = 0.05


def write(record: dict) -> None:
    """Append one count to the log."""
    if not LOG:
        return
    with open(LOG, "a", encoding="utf-8") as handle:
        handle.write(json.dumps(record) + "\n")


def main() -> int:
    session = uuid.uuid4().hex[:8]
    proc = subprocess.Popen(
        [REAL, *sys.argv[1:]],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
    )
    state = {"sent": 0, "received": 0}
    lock = threading.Lock()
    # Which request each answer belongs to: the id the client sent, and
    # what it asked for.
    asked: dict[str, dict] = {}

    def tally():
        last = (-1, -1)
        while True:
            with lock:
                sent, received = state["sent"], state["received"]
            if (sent, received) != last:
                write(
                    {
                        "session": session,
                        "argv": sys.argv[1:],
                        "sent": sent,
                        "received": received,
                    }
                )
                last = (sent, received)
            time.sleep(EVERY)

    reporter = threading.Thread(target=tally, daemon=True)
    reporter.start()

    def pump_in():
        pending = b""
        while True:
            chunk = sys.stdin.buffer.read1(65536)
            if not chunk:
                break
            with lock:
                state["sent"] += len(chunk)
            pending += chunk
            while b"\n" in pending:
                line, pending = pending.split(b"\n", 1)
                try:
                    request = json.loads(line)
                except ValueError:
                    continue
                identifier = str(request.get("id", ""))
                asked[identifier] = {
                    "method": request.get("method", ""),
                    "tool": request.get("params", {}).get("name", ""),
                }
            try:
                proc.stdin.write(chunk)
                proc.stdin.flush()
            except (BrokenPipeError, ValueError):
                break
        try:
            proc.stdin.close()
        except (BrokenPipeError, ValueError):
            pass

    feeder = threading.Thread(target=pump_in, daemon=True)
    feeder.start()
    out = sys.stdout.buffer
    pending = b""
    while True:
        chunk = proc.stdout.read1(65536)
        if not chunk:
            break
        with lock:
            state["received"] += len(chunk)
        out.write(chunk)
        out.flush()
        pending += chunk
        while b"\n" in pending:
            line, pending = pending.split(b"\n", 1)
            try:
                answer = json.loads(line)
            except ValueError:
                continue
            request = asked.get(str(answer.get("id", "")), {})
            write(
                {
                    "session": session,
                    "answer": True,
                    "method": request.get("method", ""),
                    "tool": request.get("tool", ""),
                    "bytes": len(line) + 1,
                }
            )
    code = proc.wait()
    feeder.join(timeout=1)
    with lock:
        write(
            {
                "session": session,
                "argv": sys.argv[1:],
                "sent": state["sent"],
                "received": state["received"],
                "final": True,
            }
        )
    return code


if __name__ == "__main__":
    raise SystemExit(main())
