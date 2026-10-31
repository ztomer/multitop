#!/usr/bin/env python3
"""Measure the bytes and the wall time one Ops gather moves, per host, on the
live estate.

Speaks the same protocol the monitor's client speaks (2026-07-28, stateless),
over ssh, request by request: one line in, one line out, so every reply's
byte count is the bytes that crossed the wire for THAT request.

`alerts.since` is asked with the marker the monitor really uses: now minus
the 24 h alert window.

Usage: measure_gather.py <host-label> <ssh-target> <mcp-command...>
"""

import json
import subprocess
import sys
import time

PROTO = "2026-07-28"
ALERT_WINDOW_S = 86_400


def requests(now: int):
    return [
        ("server/discover", {}),
        ("tools/list", {}),
        # The PUBLISHED verdict: the gather reads the resource, and asks the
        # suite's own list beside it (`health.list`).
        ("resources/read", {"uri": "health://latest"}),
        ("tools/call", {"name": "health.list", "arguments": {}}),
        ("tools/call", {"name": "net.egress", "arguments": {}}),
        ("tools/call", {"name": "cron.status", "arguments": {}}),
        ("tools/call", {"name": "docker.containers", "arguments": {}}),
        ("tools/call", {"name": "zfs.health", "arguments": {}}),
        ("tools/call", {"name": "disk.usage", "arguments": {}}),
        ("tools/call", {"name": "host.facts", "arguments": {}}),
        (
            "tools/call",
            {"name": "alerts.since", "arguments": {"marker": now - ALERT_WINDOW_S}},
        ),
    ]


def one_gather(target: str, command: str, now: int):
    proc = subprocess.Popen(
        ["ssh", "-o", "BatchMode=yes", target, command],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
        text=True,
        bufsize=1,
    )
    total = 0
    rows = []
    started = time.monotonic()
    for i, (method, params) in enumerate(requests(now), start=1):
        params = dict(params)
        params["_meta"] = {
            "io.modelcontextprotocol/protocolVersion": PROTO,
            "io.modelcontextprotocol/clientCapabilities": {},
        }
        req = json.dumps(
            {"jsonrpc": "2.0", "id": i, "method": method, "params": params},
            separators=(",", ":"),
        )
        t0 = time.monotonic()
        proc.stdin.write(req + "\n")
        proc.stdin.flush()
        line = proc.stdout.readline()
        dt = time.monotonic() - t0
        n = len(line.encode())
        total += n
        rows.append((method, params.get("name", ""), n, dt))
    wall = time.monotonic() - started
    proc.stdin.close()
    proc.wait(timeout=30)
    return rows, total, wall


def main() -> int:
    label, target = sys.argv[1], sys.argv[2]
    command = " ".join(sys.argv[3:])
    now = int(time.time())
    rows, total, wall = one_gather(target, command, now)
    print(f"### {label} ({target})")
    for method, name, n, dt in rows:
        print(f"  {n:>8}  B  {dt:>6.3f}s  {method}{' ' + name if name else ''}")
    print(f"  {total:>8}  B  {wall:>6.3f}s  TOTAL one gather")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
