#!/usr/bin/env python3
"""Print the `server/discover` answer from a host's own mcp_host, over ssh.

**WHY THIS PROBE IS IN THE REPO.** The tool catalogue is kept between the
poll's sessions, keyed to what determines it: the server's advertised
VERSION and the set of tools it says it does NOT offer (`dev.multitop/
absent`). Both live in the `server/discover` answer, and a reader that
wants to see what a host advertises - to check a kept list against a live
host, or to see why a host does not offer a tool - reads this answer. So
the probe is here, and its own tests are with it.

Usage: probe_discover.py <ssh-target> <mcp-command...>
"""
import json
import subprocess
import sys

PROTO = "2026-07-28"


def request() -> str:
    """The one request this probe makes, as the one line it is written on."""
    return json.dumps(
        {
            "jsonrpc": "2.0",
            "id": 1,
            "method": "server/discover",
            "params": {
                "_meta": {
                    "io.modelcontextprotocol/protocolVersion": PROTO,
                    "io.modelcontextprotocol/clientCapabilities": {},
                }
            },
        },
        separators=(",", ":"),
    )


def parse(out: str) -> dict:
    """The answer the host gave, or the reason there is none.

    The LAST line is the answer: a server may write a line of its own
    before it answers (the estate's `mcp_host` does not, but the protocol
    allows a notification), and the answer is the last thing said.
    """
    line = out.strip().splitlines()[-1] if out.strip() else ""
    try:
        return json.loads(line)
    except json.JSONDecodeError as error:
        raise ValueError(f"the host's answer was not JSON: {error}") from error


def main() -> int:
    target = sys.argv[1]
    command = " ".join(sys.argv[2:])
    out = subprocess.run(
        ["ssh", "-o", "BatchMode=yes", target, command],
        input=request() + "\n",
        capture_output=True,
        text=True,
    )
    try:
        print(json.dumps(parse(out.stdout), indent=2))
    except ValueError:
        print(out.stdout, out.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
