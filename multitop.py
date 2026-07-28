#!/usr/bin/env python3
import asyncio
import os
import shlex
import shutil
import sys
import tomllib


_SSH_BASE = [
    "-o", "ControlMaster=auto",
    "-o", "ControlPath=/tmp/multitop-ssh-%C",
    "-o", "ControlPersist=30s",
]

from rich.text import Text

from textual.app import App, ComposeResult
from textual.binding import Binding
from textual.containers import Vertical
from textual.widgets import Static

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
        height: 1fr;
    }
    ServerPanel > Static {
        margin: 0 1;
    }
    #keybar {
        background: $panel;
        color: $text-muted;
        height: 1;
        dock: bottom;
    }
    """

    BINDINGS = [
        Binding("escape", "quit", "ESC Quit"),
        Binding("d", "toggle_docker", "D"),
        Binding("u", "run_upgrade", "U"),
    ]

    def __init__(self, servers):
        super().__init__()
        self.servers = servers
        self._tasks = {}
        self._procs = {}
        self._gen = {}
        self._mode = {}  # panel -> "monitor" | "docker" | "upgrade"

    def compose(self):
        with Vertical():
            for srv in self.servers:
                yield ServerPanel(srv)
        yield Static(" ESC Quit  D Docker  U Upgrade", id="keybar")

    async def on_mount(self):
        for panel, srv in zip(self.query(ServerPanel), self.servers):
            self._tasks[panel] = asyncio.create_task(self._monitor_server(panel, srv))

    def _target(self, srv):
        user = srv.get("user", "")
        host = srv["host"]
        return f"{user}@{host}" if user else host

    def _cancel_tasks(self, *panels):
        for p in panels:
            if p in self._tasks:
                self._tasks[p].cancel()
            if p in self._procs:
                try:
                    self._procs[p].terminate()
                except Exception:
                    pass

    def _current_gen(self, panel):
        return self._gen.get(panel, 0)

    def _next_gen(self, panel):
        g = self._current_gen(panel) + 1
        self._gen[panel] = g
        return g

    async def _monitor_server(self, panel, srv):
        host = srv["host"]
        port = srv.get("port", 22)
        target = self._target(srv)
        gen = self._next_gen(panel)
        self._mode[panel] = "monitor"

        n = len(self.servers)
        term = shutil.get_terminal_size()
        panel_w = max(40, term.columns - 4)
        panel_h = max(8, ((term.lines - 2) // n) - 2)

        quoted = shlex.quote(COMPACT_MONITOR)
        cmd = f"python3 -c {quoted} {shlex.quote(host)} {panel_w} {panel_h}"

        proc = None
        try:
            proc = await asyncio.create_subprocess_exec(
                "ssh", *_SSH_BASE, target, "-p", str(port), cmd,
                stdout=asyncio.subprocess.PIPE,
                stderr=asyncio.subprocess.STDOUT,
            )
            self._procs[panel] = proc

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
        except asyncio.CancelledError:
            raise
        except FileNotFoundError:
            panel.output.update(Text.from_ansi("[red]ssh command not found[/]"))
        except Exception as e:
            panel.output.update(Text.from_ansi(f"[red]{e}[/]"))
        finally:
            if proc is not None and self._procs.get(panel) is proc:
                del self._procs[panel]

    async def _show_docker(self, panel, srv):
        self._mode[panel] = "docker"
        cmd = (
            "if command -v docker >/dev/null 2>&1; then "
            "docker ps --format 'table {{.Names}}\t{{.Status}}\t{{.Image}}' 2>&1; "
            "else echo '>> no dockers'; fi"
        )
        gen = self._next_gen(panel)
        await self._run_ssh_cmd(panel, srv, cmd, gen, "Docker")
        if self._current_gen(panel) == gen:
            await self._restart_monitor(panel, srv)

    async def _run_upgrade(self, panel, srv):
        self._mode[panel] = "upgrade"
        cmd = srv.get("upgrade_cmd", "")
        gen = self._next_gen(panel)

        if not cmd:
            panel.output.update(
                Text.from_ansi("[yellow]No upgrade_cmd configured for this server[/]")
            )
            await asyncio.sleep(5)
            if self._current_gen(panel) == gen:
                await self._restart_monitor(panel, srv)
            return

        await self._run_ssh_cmd(panel, srv, cmd, gen, "Upgrade")
        if self._current_gen(panel) == gen:
            await self._restart_monitor(panel, srv)

    async def _run_ssh_cmd(self, panel, srv, cmd, gen, label):
        host = srv["host"]
        port = srv.get("port", 22)
        target = self._target(srv)

        proc = None
        try:
            proc = await asyncio.create_subprocess_exec(
                "ssh", *_SSH_BASE, target, "-p", str(port), cmd,
                stdout=asyncio.subprocess.PIPE,
                stderr=asyncio.subprocess.STDOUT,
            )
            self._procs[panel] = proc

            lines = [f"[bold]{label} on {host}[/]"]
            while True:
                raw = await proc.stdout.readline()
                if not raw:
                    break
                lines.append(raw.decode("utf-8", errors="replace").rstrip())
            await proc.wait()
            if self._current_gen(panel) == gen:
                panel.output.update(Text.from_ansi("\n".join(lines)))
        except asyncio.CancelledError:
            raise
        except Exception as e:
            if self._current_gen(panel) == gen:
                panel.output.update(Text.from_ansi(f"[red]{e}[/]"))
        finally:
            if proc is not None and self._procs.get(panel) is proc:
                del self._procs[panel]

    async def _restart_monitor(self, panel, srv):
        self._tasks[panel] = asyncio.create_task(self._monitor_server(panel, srv))

    async def action_toggle_docker(self):
        panels = list(self.query(ServerPanel))
        servers = self.servers
        in_docker = any(self._mode.get(p) == "docker" for p in panels)
        self._cancel_tasks(*panels)
        for panel, srv in zip(panels, servers):
            if in_docker:
                self._tasks[panel] = asyncio.create_task(self._monitor_server(panel, srv))
            else:
                self._tasks[panel] = asyncio.create_task(self._show_docker(panel, srv))

    async def action_run_upgrade(self):
        panels = list(self.query(ServerPanel))
        servers = self.servers
        self._cancel_tasks(*panels)
        for panel, srv in zip(panels, servers):
            self._tasks[panel] = asyncio.create_task(self._run_upgrade(panel, srv))

    async def action_quit(self) -> None:
        for proc in self._procs.values():
            try:
                proc.terminate()
            except Exception:
                pass
        for task in self._tasks.values():
            try:
                task.cancel()
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
