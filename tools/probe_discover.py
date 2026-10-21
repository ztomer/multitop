#!/usr/bin/env python3
"""Print the `server/discover` answer from a host's own mcp_host, over ssh.

Usage: probe_discover.py <ssh-target> <mcp-command...>
"""
import json
import subprocess
import sys

PROTO = "2026-07-28"


def main() -> int:
    target = sys.argv[1]
    command = " ".join(sys.argv[2:])
    req = json.dumps(
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
    out = subprocess.run(
        ["ssh", "-o", "BatchMode=yes", target, command],
        input=req + "\n",
        capture_output=True,
        text=True,
    )
    line = out.stdout.strip().splitlines()[-1] if out.stdout.strip() else ""
    try:
        d = json.loads(line)
    except json.JSONDecodeError:
        print(out.stdout, out.stderr)
        return 1
    print(json.dumps(d, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
