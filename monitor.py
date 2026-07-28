#!/usr/bin/env python3
import os
import shlex
import shutil
import sys
import subprocess
import tomllib

CONFIG_PATH = os.path.expanduser("~/.config/monitor/config.toml")
SESSION_NAME = "multi_server_monitor"


def _compact_monitor_source():
    script = os.path.join(os.path.dirname(os.path.abspath(__file__)), "compact_monitor.py")
    with open(script) as f:
        return f.read()


COMPACT_MONITOR = _compact_monitor_source()


class MonitorError(Exception):
    pass


def require_commands(*cmds):
    missing = [c for c in cmds if shutil.which(c) is None]
    if missing:
        raise MonitorError(
            f"Missing required commands: {' '.join(missing)}"
        )


def build_ssh_cmd(target, port):
    quoted = shlex.quote(COMPACT_MONITOR)
    return ["ssh", "-t", target, "-p", str(port), f"python3 -c {quoted}"]


def validate_user(user):
    if not user:
        return user
    if any(c.isspace() for c in user):
        raise MonitorError(
            f"Invalid user '{user}': contains whitespace"
        )
    return user


def kill_existing_session():
    result = subprocess.run(
        ["tmux", "has-session", "-t", SESSION_NAME],
        capture_output=True,
    )
    if result.returncode != 0:
        return
    print(
        "[Warn] Killing existing session with active clients.",
        file=sys.stderr,
    )
    try:
        subprocess.run(
            ["tmux", "kill-session", "-t", SESSION_NAME],
            check=True,
        )
    except subprocess.CalledProcessError as e:
        print(f"[Error] Failed to kill existing session: {e}", file=sys.stderr)


def teardown_session():
    subprocess.run(["tmux", "unbind-key", "-n", "Escape"])
    subprocess.run(
        ["tmux", "set-option", "-t", SESSION_NAME, "status-left", ""],
    )


def parse_toml_servers(path):
    if not os.path.exists(path):
        raise MonitorError(f"Configuration file missing at {path}")

    with open(path, "rb") as f:
        cfg = tomllib.load(f)

    servers = cfg.get("servers", [])
    if not isinstance(servers, list):
        raise MonitorError(
            "'servers' must be a list of tables, got a non-list value"
        )

    if not servers:
        raise MonitorError("No 'servers' entries found in configuration")

    for idx, s in enumerate(servers):
        if not isinstance(s, dict):
            raise MonitorError(
                f"Server entry at index {idx} is not a table"
            )
        host = s.get("host")
        if not host:
            raise MonitorError(
                f"Server entry at index {idx} missing 'host' field"
            )
        validate_user(s.get("user", ""))

    return servers


def get_ssh_target(server):
    user = server.get("user", "")
    host = server["host"]
    port = server.get("port", 22)
    prefix = f"{user}@" if user else ""
    return prefix + host, port


def create_session(servers):
    target, port = get_ssh_target(servers[0])
    subprocess.run(
        ["tmux", "new-session", "-d", "-s", SESSION_NAME]
        + build_ssh_cmd(target, port),
        check=True,
    )

    for srv in servers[1:]:
        target, port = get_ssh_target(srv)
        subprocess.run(
            ["tmux", "split-window", "-v", "-t", SESSION_NAME]
            + build_ssh_cmd(target, port),
            check=True,
        )

    subprocess.run(
        ["tmux", "select-layout", "-t", SESSION_NAME, "even-vertical"],
        check=True,
    )


def bind_exit_keys():
    subprocess.run(
        ["tmux", "bind-key", "-n", "Escape", "kill-session"],
        check=True,
    )
    subprocess.run(
        ["tmux", "set-option", "-t", SESSION_NAME, "status-left", "[ESC Exits] "],
        check=True,
    )
    subprocess.run(
        ["tmux", "set-option", "-t", SESSION_NAME, "status-left-length", "14"],
        check=True,
    )


def main():
    try:
        require_commands("tmux", "ssh")
        servers = parse_toml_servers(CONFIG_PATH)
        kill_existing_session()

        try:
            create_session(servers)
            bind_exit_keys()
        except subprocess.CalledProcessError:
            teardown_session()
            kill_existing_session()
            raise MonitorError("Session setup failed")

        print(
            f"[Info] Monitoring {len(servers)} servers in tmux session '{SESSION_NAME}'",
            file=sys.stderr,
        )

        try:
            subprocess.run(
                ["tmux", "attach-session", "-t", SESSION_NAME],
                check=True,
            )
        except subprocess.CalledProcessError as e:
            print(f"[Error] Attach failed: {e}", file=sys.stderr)
        finally:
            teardown_session()

    except MonitorError as e:
        print(f"[Error] {e}", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
