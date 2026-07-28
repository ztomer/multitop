#!/usr/bin/env python3
import asyncio
import os
import shlex
import shutil
import sys
import tomllib

from rich.text import Text

from textual.app import App, ComposeResult
from textual.binding import Binding
from textual.containers import Vertical
from textual.widgets import Header, Footer, Static

CONFIG_PATH = os.path.expanduser("~/.config/multitop/config.toml")
_EXAMPLE_CONFIG = os.path.join(os.path.dirname(os.path.abspath(__file__)), "config.example.toml")


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
        raise MonitorError(f"Missing required commands: {' '.join(missing)}")


def validate_user(user):
    if not user:
        return user
    if any(c.isspace() for c in user):
        raise MonitorError(f"Invalid user '{user}': contains whitespace")
    return user


def parse_toml_servers(path):
    if not os.path.exists(path):
        old = os.path.expanduser("~/.config/monitor/config.toml")
        if os.path.exists(old):
            raise MonitorError(
                f"Configuration file missing at {path}\n\n"
                f"  Your config is still at the old location:\n"
                f"    {old}\n\n"
                f"  Migrate it:\n"
                f"    mkdir -p ~/.config/multitop\n"
                f"    mv {old} {path}"
            )
        example = _EXAMPLE_CONFIG if os.path.exists(_EXAMPLE_CONFIG) else ""
        hint = ""
        if example:
            with open(example) as f:
                hint = "\n".join("  " + l.rstrip() for l in f if l.strip())
        raise MonitorError(
            f"Configuration file missing at {path}\n\n"
            f"  Create it. Example:\n\n{hint}\n"
        )
    with open(path, "rb") as f:
        cfg = tomllib.load(f)
    servers = cfg.get("servers", [])
    if not isinstance(servers, list):
        raise MonitorError("'servers' must be a list of tables, got a non-list value")
    if not servers:
        raise MonitorError("No 'servers' entries found in configuration")
    for idx, s in enumerate(servers):
        if not isinstance(s, dict):
            raise MonitorError(f"Server entry at index {idx} is not a table")
        host = s.get("host")
        if not host:
            raise MonitorError(f"Server entry at index {idx} missing 'host' field")
        validate_user(s.get("user", ""))
    return servers


class ServerPanel(Vertical):
    def __init__(self, server_cfg):
        super().__init__()
        self.border_title = server_cfg["host"]
        self.output = Static("connecting...")

    def compose(self):
        yield self.output


class MonitorApp(App):
    CSS = """
    Screen {
        layout: vertical;
    }
    Vertical {
        height: 1fr;
    }
    ServerPanel {
        border: solid $primary;
        height: 1fr;
    }
    ServerPanel > Static {
        margin: 0 1;
    }
    """

    BINDINGS = [
        Binding("escape", "quit", "Exit"),
    ]

    def __init__(self, servers):
        super().__init__()
        self.servers = servers
        self._ssh_procs = []

    def compose(self):
        yield Header(show_clock=False)
        with Vertical():
            for srv in self.servers:
                yield ServerPanel(srv)
        yield Footer()

    async def on_mount(self):
        for panel, srv in zip(self.query(ServerPanel), self.servers):
            asyncio.create_task(self._monitor_server(panel, srv))

    async def _monitor_server(self, panel, srv):
        host = srv["host"]
        port = srv.get("port", 22)
        user = srv.get("user", "")
        target = f"{user}@{host}" if user else host

        quoted = shlex.quote(COMPACT_MONITOR)
        cmd = f"python3 -c {quoted} {shlex.quote(host)}"

        try:
            proc = await asyncio.create_subprocess_exec(
                "ssh", target, "-p", str(port), cmd,
                stdout=asyncio.subprocess.PIPE,
                stderr=asyncio.subprocess.STDOUT,
            )
            self._ssh_procs.append(proc)

            buf = []
            while True:
                raw = await proc.stdout.readline()
                if not raw:
                    break
                line = raw.decode("utf-8", errors="replace").rstrip()
                if line == "===MONITOR===":
                    if buf:
                        panel.output.update(Text.from_ansi("\n".join(buf)))
                    buf = []
                else:
                    buf.append(line)
            if buf:
                panel.output.update(Text.from_ansi("\n".join(buf)))
        except FileNotFoundError:
            panel.output.update(Text.from_ansi("[red]ssh command not found[/]"))
        except Exception as e:
            panel.output.update(Text.from_ansi(f"[red]{e}[/]"))

    async def action_quit(self) -> None:
        for proc in self._ssh_procs:
            try:
                proc.terminate()
            except Exception:
                pass
        self.exit()


def main():
    try:
        require_commands("ssh")
        servers = parse_toml_servers(CONFIG_PATH)
    except MonitorError as e:
        print(f"[Error] {e}", file=sys.stderr)
        sys.exit(1)

    app = MonitorApp(servers)
    app.run()


if __name__ == "__main__":  # pragma: no cover
    main()