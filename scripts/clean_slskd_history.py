#!/usr/bin/env python3
"""Clear all slskd search history via the API in batches."""

import argparse
import json
import sys
import urllib.error
import urllib.request


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--host", default="192.168.0.33", help="slskd host")
    ap.add_argument("--port", default=5030, type=int, help="slskd web port")
    ap.add_argument("--api-key", default="", help="slskd API key (or set SLSKD_API_KEY)")
    ap.add_argument("--batch", default=100, type=int, help="fetch/delete batch size")
    args = ap.parse_args()

    api_key = args.api_key or ""
    if not api_key:
        print("error: provide --api-key", file=sys.stderr)
        sys.exit(1)

    base = f"http://{args.host}:{args.port}"
    headers = {"X-API-Key": api_key}

    def req(method, path):
        r = urllib.request.Request(base + path, method=method, headers=headers)
        try:
            with urllib.request.urlopen(r, timeout=30) as resp:
                return resp.status, resp.read()
        except urllib.error.HTTPError as e:
            return e.code, e.read()

    deleted = 0
    while True:
        status, body = req("GET", f"/api/v0/searches?limit={args.batch}")
        if status != 200:
            print(f"error: list returned {status}", file=sys.stderr)
            sys.exit(1)
        searches = json.loads(body)
        if not searches:
            break
        for s in searches:
            st, _ = req("DELETE", f"/api/v0/searches/{s['id']}")
            if st == 204:
                deleted += 1
            else:
                print(f"FAIL delete {s['id']}: {st}", file=sys.stderr)
        print(f"deleted {deleted}...", file=sys.stderr)

    print(deleted)


if __name__ == "__main__":
    main()
