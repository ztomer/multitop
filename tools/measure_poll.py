#!/usr/bin/env python3
"""Measure what the SHIPPED Ops poll pulls off each host, a pass at a time.

`tools/measure_gather.py` measures one gather's ANSWERS, by replaying the
requests a gather makes against a host over ssh. This measures the other
half: the bytes the monitor's own poll really moves, by running the
shipped binary with a counting shim in front of `ssh` (see
`tools/ssh_bytes.py`), with the kept TOOL LISTS cold and warm - so the
difference between the two runs is what the kept lists save, measured on
the binary that ships, not on a replay of it.

The hosts are made DUE first - the poll only asks a host whose answer is
missing or older than the poll's own interval - by removing the answers
the poll kept. They are the poll's own cache, and the pass writes them
again as it goes.

Usage:
  measure_poll.py --binary PATH [--label NAME] [--every SECONDS]
                  [--drop-catalogue]

  --drop-catalogue   remove the kept TOOL LISTS before the pass, so the
                     pass asks each host for its list again (the cold case)

The kept lists live in the cache directory's own `catalogue/` directory
(servers ROADMAP E61); the answers live beside it, one file per host.
"""
import argparse
import json
import os
import pathlib
import shutil
import subprocess
import sys
import tempfile

HERE = pathlib.Path(__file__).resolve().parent
CONFIG_DIR = pathlib.Path.home() / ".config" / "multitop"
OPS_DIR = CONFIG_DIR / "ops"
CATALOGUE_DIR = OPS_DIR / "catalogue"
# The scheduler's own interval: the poll runs this often (the launchd
# agent's StartInterval).
EVERY = 390


def force_due() -> int:
    """Remove the answers the poll kept, so every host is due."""
    removed = 0
    if not OPS_DIR.is_dir():
        return removed
    for path in OPS_DIR.glob("*.json"):
        path.unlink()
        removed += 1
    return removed


def drop_catalogue() -> int:
    """Remove the kept tool lists, so the cold case can be measured."""
    removed = 0
    if not CATALOGUE_DIR.is_dir():
        return removed
    for path in CATALOGUE_DIR.glob("*.json"):
        path.unlink()
        removed += 1
    return removed


def run_pass(binary: str) -> tuple[list[dict], str]:
    """Run one pass with the counting shim in front of ssh."""
    shim_dir = tempfile.mkdtemp(prefix="multitop-measure-")
    log = os.path.join(shim_dir, "sessions.jsonl")
    # The real ssh, found BEFORE the shim is put on the PATH: otherwise the
    # shim would find itself and the session would be the shim talking to
    # the shim.
    real_ssh = shutil.which("ssh") or "/usr/bin/ssh"
    try:
        shim = pathlib.Path(shim_dir) / "ssh"
        shutil.copy(HERE / "ssh_bytes.py", shim)
        shim.chmod(0o755)
        env = dict(os.environ)
        env["PATH"] = f"{shim_dir}:{env['PATH']}"
        env["MEASURE_SSH_REAL"] = real_ssh
        env["MEASURE_SSH_LOG"] = log
        done = subprocess.run(
            [binary, "--ops-poll"],
            env=env,
            capture_output=True,
            text=True,
            stdin=subprocess.DEVNULL,
            check=False,
        )
        if done.returncode != 0:
            # A failed pass is not a measurement: reporting its (empty or partial) log as one
            # read as "this poll costs nothing" (goh G17b).
            raise SystemExit(f"measure-poll: --ops-poll exited {done.returncode}\n{done.stdout}{done.stderr}")
        records = []
        if os.path.exists(log):
            with open(log, encoding="utf-8") as handle:
                records = [json.loads(line) for line in handle if line.strip()]
        return records, done.stdout + done.stderr
    finally:
        shutil.rmtree(shim_dir, ignore_errors=True)


def target_of(argv: list[str]) -> str:
    """The host an ssh invocation names."""
    for i, arg in enumerate(argv):
        if arg == "-p" and i + 2 < len(argv):
            return f"{argv[i + 2]}:{argv[i + 1]}"
    return argv[-2] if len(argv) >= 2 else "?"


def report(records: list[dict], label: str, every: int) -> None:
    """Print what the pass cost, a gather at a time and a day at a time."""
    # One session per host: the tally is written out as it grows, so the
    # last count of a session is the session's count.
    per_session: dict[str, dict] = {}
    answers: dict[str, dict[str, int]] = {}
    for record in records:
        session = record["session"]
        if record.get("answer"):
            held = answers.setdefault(session, {})
            name = record.get("tool") or record.get("method") or "?"
            held[name] = held.get(name, 0) + record["bytes"]
            continue
        held = per_session.setdefault(
            session, {"argv": record["argv"], "received": 0, "sent": 0}
        )
        held["received"] = max(held["received"], record["received"])
        held["sent"] = max(held["sent"], record["sent"])
    per_host: dict[str, int] = {}
    per_host_answers: dict[str, dict[str, int]] = {}
    for session, held in per_session.items():
        host = target_of(held["argv"])
        per_host[host] = per_host.get(host, 0) + held["received"]
        per_host_answers[host] = answers.get(session, {})
    # **THE COUNTED ANSWERS ARE THE COUNT, NOT THE TALLY.** The tally is
    # written out as it grows, and the client kills the session's process
    # group when the pass is over - so the tally's last write can be a few
    # milliseconds BEFORE the last answer arrives, and that answer's bytes
    # are missing from it. The per-answer counts are written as each answer
    # is forwarded, so their sum is every byte the host sent.
    counted = {host: sum(held.values()) for host, held in per_host_answers.items()}
    a_day = 86_400 // every
    print(f"\n=== {label} ===")
    print(f"{'host':<28}{'bytes a gather':>16}{'a day':>14}")
    total = 0
    for host in sorted(per_host):
        got = counted.get(host, per_host[host])
        total += got
        print(f"{host:<28}{got:>16,}{got * a_day:>14,}")
    print(f"{'ALL':<28}{total:>16,}{total * a_day:>14,}")
    print(f"({a_day} passes a day, one every {every}s)")
    print("\nwhat each answer cost, a gather:")
    for host in sorted(per_host_answers):
        for name, size in sorted(
            per_host_answers[host].items(), key=lambda kv: -kv[1]
        ):
            print(f"  {host:<28}{name:<20}{size:>10,}")
    return total


def totals(records: list[dict]) -> dict:
    """The per-host totals of a pass, for comparing two passes."""
    per_session: dict[str, dict] = {}
    answers: dict[str, dict[str, int]] = {}
    for record in records:
        session = record["session"]
        if record.get("answer"):
            held = answers.setdefault(session, {})
            name = record.get("tool") or record.get("method") or "?"
            held[name] = held.get(name, 0) + record["bytes"]
            continue
        held = per_session.setdefault(
            session, {"argv": record["argv"], "received": 0, "sent": 0}
        )
        held["received"] = max(held["received"], record["received"])
        held["sent"] = max(held["sent"], record["sent"])
    per_host: dict[str, int] = {}
    for session, held in per_session.items():
        host = target_of(held["argv"])
        counted = sum(answers.get(session, {}).values())
        # The counted answers, not the tally: the tally's last write can be
        # a few milliseconds before the last answer arrives, and the client
        # kills the session's process group when the pass is over.
        per_host[host] = per_host.get(host, 0) + (counted or held["received"])
    return per_host


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, help="the monitor binary to run")
    parser.add_argument("--label", default="pass", help="what this pass is called")
    parser.add_argument("--every", type=int, default=EVERY, help="seconds between passes")
    parser.add_argument(
        "--drop-catalogue",
        action="store_true",
        help="remove the kept tool lists first, so the pass asks for the list again",
    )
    args = parser.parse_args()

    removed = force_due()
    dropped = drop_catalogue() if args.drop_catalogue else 0
    print(f"made {removed} hosts due; dropped {dropped} kept tool lists")
    records, out = run_pass(args.binary)
    if out.strip():
        print(out.strip())
    report(records, args.label, args.every)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
