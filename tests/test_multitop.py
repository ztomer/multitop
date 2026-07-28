import os
import sys
import runpy
from unittest.mock import AsyncMock, MagicMock, patch

import pytest

from multitop import (
    CONFIG_PATH,
    COMPACT_MONITOR,
    MonitorApp,
    MonitorError,
    ServerPanel,
    _compact_monitor_source,
    main,
    parse_toml_servers,
    require_commands,
    validate_user,
)
from textual.widgets import Static


class TestValidateUser:
    def test_empty_string(self):
        assert validate_user("") == ""

    def test_valid_simple(self):
        assert validate_user("admin") == "admin"

    def test_valid_with_hyphen(self):
        assert validate_user("monitor-user") == "monitor-user"

    def test_whitespace_raises(self):
        with pytest.raises(MonitorError, match="whitespace"):
            validate_user("bad user")

    def test_leading_space_raises(self):
        with pytest.raises(MonitorError, match="whitespace"):
            validate_user(" admin")

    def test_trailing_space_raises(self):
        with pytest.raises(MonitorError, match="whitespace"):
            validate_user("admin ")


class TestRequireCommands:
    def test_all_present(self, monkeypatch):
        monkeypatch.setattr("shutil.which", lambda c: "/usr/bin/" + c)
        require_commands("tmux", "ssh")

    def test_single_missing_raises(self, monkeypatch):
        monkeypatch.setattr("shutil.which", lambda c: None)
        with pytest.raises(MonitorError, match="Missing required"):
            require_commands("missing_cmd_xyz")

    def test_partial_missing_raises(self, monkeypatch):
        def fake_which(cmd):
            return "/bin/" + cmd if cmd == "ssh" else None

        monkeypatch.setattr("shutil.which", fake_which)
        with pytest.raises(MonitorError, match="tmux"):
            require_commands("tmux", "ssh")


class TestParseTomlServers:
    def test_valid_config(self, tmp_toml):
        path = tmp_toml("""
[[servers]]
host = "192.168.0.33"
port = 22
user = ""
""")
        result = parse_toml_servers(path)
        assert len(result) == 1
        assert result[0]["host"] == "192.168.0.33"

    def test_multi_server(self, tmp_toml):
        path = tmp_toml("""
[[servers]]
host = "192.168.0.33"

[[servers]]
host = "192.168.0.90"
user = "admin"
""")
        result = parse_toml_servers(path)
        assert len(result) == 2
        assert result[1]["user"] == "admin"

    def test_missing_file_raises(self, tmp_path):
        with pytest.raises(MonitorError, match="missing"):
            parse_toml_servers(str(tmp_path / "nonexistent.toml"))

    def test_servers_not_list_raises(self, tmp_toml):
        path = tmp_toml("servers = {}")
        with pytest.raises(MonitorError, match="non-list"):
            parse_toml_servers(path)

    def test_empty_servers_raises(self, tmp_toml):
        path = tmp_toml("servers = []")
        with pytest.raises(MonitorError, match="No 'servers' entries"):
            parse_toml_servers(path)

    def test_missing_host_raises(self, tmp_toml):
        path = tmp_toml("[[servers]]\nport = 22\n")
        with pytest.raises(MonitorError, match="missing 'host'"):
            parse_toml_servers(path)

    def test_non_dict_entry_raises(self, tmp_toml):
        path = tmp_toml("servers = ['not-a-table']")
        with pytest.raises(MonitorError, match="not a table"):
            parse_toml_servers(path)

    def test_whitespace_user_raises(self, tmp_toml):
        path = tmp_toml("""
[[servers]]
host = "192.168.0.33"
user = "bad user"
""")
        with pytest.raises(MonitorError, match="whitespace"):
            parse_toml_servers(path)

    def test_second_entry_non_dict_raises(self, tmp_path):
        path = tmp_path / "test.toml"
        path.write_text("servers = [{host = 'a'}, 'bad-string']")
        with pytest.raises(MonitorError, match="not a table"):
            parse_toml_servers(str(path))


class TestIntegration:
    def test_full_config_parse(self):
        path = os.path.expanduser("~/.config/multitop/config.toml")
        if not os.path.exists(path):
            pytest.skip("No real config file available at ~/.config/multitop/config.toml")
        result = parse_toml_servers(path)
        assert len(result) >= 1
        for s in result:
            assert "host" in s


class TestCompactMonitorSource:
    def test_returns_string(self):
        src = _compact_monitor_source()
        assert isinstance(src, str)
        assert len(src) > 100
        assert "read_proc" in src
        assert "def loop" in src

    def test_global_is_set(self):
        assert isinstance(COMPACT_MONITOR, str)
        assert "loop" in COMPACT_MONITOR


class TestServerPanel:
    def test_creation(self):
        cfg = {"host": "192.168.0.33", "port": 22}
        panel = ServerPanel(cfg)
        assert panel.border_title == "192.168.0.33"
        assert isinstance(panel.output, Static)
        assert panel.output.content == "connecting..."

    def test_with_user(self):
        cfg = {"host": "10.0.0.1", "user": "admin"}
        panel = ServerPanel(cfg)
        assert panel.border_title == "10.0.0.1"

    def test_update_output(self):
        cfg = {"host": "test"}
        panel = ServerPanel(cfg)
        panel.output.update("line1\nline2\nline3")
        assert panel.output.content == "line1\nline2\nline3"


class TestMonitorApp:
    def test_creation(self):
        servers = [{"host": "a"}, {"host": "b"}]
        app = MonitorApp(servers)
        assert len(app.servers) == 2
        assert app._ssh_procs == []

    def test_empty_servers(self):
        app = MonitorApp([])
        assert app.servers == []

    async def test_mount_creates_panels(self):
        servers = [{"host": "a"}, {"host": "b"}]
        app = MonitorApp(servers)
        async with app.run_test() as pilot:
            panels = app.query(ServerPanel)
            assert len(panels) == 2
            assert panels[0].border_title == "a"
            assert panels[1].border_title == "b"

    async def test_panel_shows_connecting_initially(self):
        servers = [{"host": "test"}]
        app = MonitorApp(servers)
        async with app.run_test() as pilot:
            panel = app.query(ServerPanel).first()
            assert panel.output.content == "connecting..."

    async def test_quit_action_cleans_up(self):
        servers = [{"host": "a"}]
        app = MonitorApp(servers)
        mock_proc = MagicMock()
        app._ssh_procs.append(mock_proc)
        async with app.run_test():
            await app.action_quit()
        mock_proc.terminate.assert_called_once()

    async def test_monitor_server_ssh_not_found(self):
        with patch("asyncio.create_subprocess_exec", side_effect=FileNotFoundError):
            servers = [{"host": "x"}]
            app = MonitorApp(servers)
            async with app.run_test():
                panels = app.query(ServerPanel)
                assert len(panels) == 1

    async def test_monitor_server_generic_exception(self):
        with patch("asyncio.create_subprocess_exec", side_effect=PermissionError("denied")):
            servers = [{"host": "x"}]
            app = MonitorApp(servers)
            async with app.run_test():
                panels = app.query(ServerPanel)
                assert len(panels) == 1

    async def test_quit_with_failing_proc(self):
        servers = [{"host": "a"}]
        app = MonitorApp(servers)
        failing_proc = MagicMock()
        failing_proc.terminate.side_effect = OSError("no such process")
        app._ssh_procs.append(failing_proc)
        async with app.run_test():
            await app.action_quit()
        failing_proc.terminate.assert_called_once()

    async def test_data_before_marker(self):
        def make_proc(*lines):
            reader = AsyncMock()
            reader.readline.side_effect = lines
            proc = MagicMock()
            proc.stdout = reader
            return proc
        proc = make_proc(b"line1\n", b"line2\n", b"===MONITOR===\n", b"")
        with patch("asyncio.create_subprocess_exec",
                   new_callable=AsyncMock, return_value=proc):
            servers = [{"host": "x"}]
            app = MonitorApp(servers)
            async with app.run_test() as pilot:
                await pilot.pause()
                panel = app.query(ServerPanel).first()
                assert "line1" in panel.output.content
                assert "line2" in panel.output.content

    async def test_marker_without_data(self):
        def make_proc(*lines):
            reader = AsyncMock()
            reader.readline.side_effect = lines
            proc = MagicMock()
            proc.stdout = reader
            return proc
        proc = make_proc(b"===MONITOR===\n", b"")
        with patch("asyncio.create_subprocess_exec",
                   new_callable=AsyncMock, return_value=proc):
            servers = [{"host": "x"}]
            app = MonitorApp(servers)
            async with app.run_test() as pilot:
                await pilot.pause()
                panel = app.query(ServerPanel).first()
                assert panel.output.content == "connecting..."

    async def test_no_data_after_start(self):
        proc = AsyncMock()
        proc.stdout.readline.side_effect = [b""]
        with patch("asyncio.create_subprocess_exec",
                   new_callable=AsyncMock, return_value=proc):
            servers = [{"host": "x"}]
            app = MonitorApp(servers)
            async with app.run_test() as pilot:
                await pilot.pause()
                panel = app.query(ServerPanel).first()
                assert panel.output.content == "connecting..."


class TestMain:
    def test_missing_config_exits(self, monkeypatch, tmp_path):
        config_path = str(tmp_path / "nonexistent.toml")
        monkeypatch.setattr("multitop.CONFIG_PATH", config_path)
        with pytest.raises(SystemExit):
            main()

    def test_missing_command_exits(self, monkeypatch, tmp_path):
        p = tmp_path / "config.toml"
        p.write_text("[[servers]]\nhost = '192.168.0.33'\n")
        monkeypatch.setattr("multitop.CONFIG_PATH", str(p))
        monkeypatch.setattr("shutil.which", lambda c: None)
        with pytest.raises(SystemExit):
            main()

    def test_ssh_missing_message(self, monkeypatch, tmp_path, capsys):
        p = tmp_path / "config.toml"
        p.write_text("[[servers]]\nhost = '192.168.0.33'\n")
        monkeypatch.setattr("multitop.CONFIG_PATH", str(p))
        monkeypatch.setattr("shutil.which", lambda c: None)
        with pytest.raises(SystemExit):
            main()
        captured = capsys.readouterr()
        assert "ssh" in captured.err

    def test_main_success_starts_app(self, monkeypatch, tmp_path):
        started = []
        class FakeApp:
            def __init__(self, servers):
                self.servers = servers
            def run(self):
                started.append(True)
        p = tmp_path / "config.toml"
        p.write_text("[[servers]]\nhost = '192.168.0.33'\n")
        monkeypatch.setattr("multitop.CONFIG_PATH", str(p))
        monkeypatch.setattr("multitop.MonitorApp", FakeApp)
        main()
        assert len(started) == 1

    def test_import_does_not_call_main(self):
        result = os.system(f'{sys.executable} -c "import multitop" 2>/dev/null')
        assert result == 0