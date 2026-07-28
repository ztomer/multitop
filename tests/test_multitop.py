import asyncio
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

    def test_missing_suggests_old_migration(self, monkeypatch, tmp_path):
        new_path = tmp_path / "new.toml"
        old_path = str(tmp_path / "old" / "config.toml")
        monkeypatch.setattr("multitop.os.path.expanduser", lambda p: old_path)
        monkeypatch.setattr("multitop.os.path.exists", lambda p: p in (old_path, "/nonexistent"))
        monkeypatch.setattr("multitop._EXAMPLE_CONFIG", "/nonexistent")
        with pytest.raises(MonitorError, match="old location"):
            parse_toml_servers(str(new_path))

    def test_missing_shows_example(self, monkeypatch, tmp_path):
        example = tmp_path / "example.toml"
        example.write_text("[[servers]]\nhost = '10.0.0.1'\n")
        new_path = tmp_path / "new.toml"
        monkeypatch.setattr("multitop._EXAMPLE_CONFIG", str(example))
        monkeypatch.setattr("multitop.os.path.exists", lambda p: p == str(example))
        with pytest.raises(MonitorError, match="10.0.0.1"):
            parse_toml_servers(str(new_path))

    def test_missing_no_old_no_example(self, monkeypatch, tmp_path):
        new_path = tmp_path / "new.toml"
        monkeypatch.setattr("multitop._EXAMPLE_CONFIG", "")
        monkeypatch.setattr("multitop.os.path.exists", lambda p: False)
        with pytest.raises(MonitorError, match="Example"):
            parse_toml_servers(str(new_path))


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
        assert isinstance(panel.output, Static)
        assert panel.output.content == "connecting..."

    def test_with_user(self):
        cfg = {"host": "10.0.0.1", "user": "admin"}
        panel = ServerPanel(cfg)
        assert isinstance(panel.output, Static)

    def test_update_output(self):
        cfg = {"host": "test"}
        panel = ServerPanel(cfg)
        panel.output.update("line1\nline2\nline3")
        assert panel.output.content == "line1\nline2\nline3"


def make_proc(*lines):
    reader = AsyncMock()
    reader.readline.side_effect = lines
    proc = MagicMock()
    proc.stdout = reader
    proc.wait = AsyncMock()
    return proc


class TestMonitorApp:
    def test_creation(self):
        servers = [{"host": "a"}, {"host": "b"}]
        app = MonitorApp(servers)
        assert len(app.servers) == 2
        assert app._procs == {}
        assert app._aux_procs == {}
        assert app._frames == {}

    def test_empty_servers(self):
        app = MonitorApp([])
        assert app.servers == []

    async def test_mount_creates_panels(self):
        servers = [{"host": "a"}, {"host": "b"}]
        app = MonitorApp(servers)
        async with app.run_test() as pilot:
            panels = app.query(ServerPanel)
            assert len(panels) == 2

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
        app._procs[MagicMock()] = mock_proc
        app._aux_procs[MagicMock()] = mock_proc
        async with app.run_test():
            await app.action_quit()
        assert mock_proc.terminate.call_count == 2

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
        app._procs[MagicMock()] = failing_proc
        app._aux_procs[MagicMock()] = failing_proc
        async with app.run_test():
            await app.action_quit()
        assert failing_proc.terminate.call_count == 2

    async def test_data_before_marker(self):
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

    # --- gen / cancel / mode helpers ---

    def test_current_gen_defaults_zero(self):
        app = MonitorApp([])
        assert app._current_gen("x") == 0

    def test_next_gen_increments(self):
        app = MonitorApp([])
        assert app._next_gen("x") == 1
        assert app._next_gen("x") == 2
        assert app._gen["x"] == 2

    def test_target_no_user(self):
        app = MonitorApp([])
        assert app._target({"host": "10.0.0.1"}) == "10.0.0.1"

    def test_target_with_user(self):
        app = MonitorApp([])
        assert app._target({"host": "10.0.0.1", "user": "admin"}) == "admin@10.0.0.1"

    # --- action_toggle_docker ---

    async def test_toggle_docker_starts_docker_task(self):
        servers = [{"host": "a"}]
        app = MonitorApp(servers)
        async with app.run_test() as pilot:
            panel = app.query(ServerPanel).first()
            with patch.object(MonitorApp, "on_mount", new=AsyncMock()):
                await pilot.pause()
                await app.action_toggle_docker()
                await pilot.pause()
                assert app._mode.get(panel) == "docker"

    async def test_toggle_docker_returns_to_monitor(self):
        servers = [{"host": "a"}]
        app = MonitorApp(servers)
        async with app.run_test() as pilot:
            panel = app.query(ServerPanel).first()
            app._mode[panel] = "docker"
            await app.action_toggle_docker()
            await pilot.pause()
            assert app._mode.get(panel) == "monitor"

    # --- action_switch_stats ---

    async def test_switch_stats_sets_monitor_mode(self):
        servers = [{"host": "a"}]
        app = MonitorApp(servers)
        async with app.run_test() as pilot:
            panel = app.query(ServerPanel).first()
            app._mode[panel] = "docker"
            await app.action_switch_stats()
            await pilot.pause()
            assert app._mode.get(panel) == "monitor"

    async def test_switch_stats_shows_frame_when_available(self):
        servers = [{"host": "a"}]
        app = MonitorApp(servers)
        with patch.object(MonitorApp, "on_mount", new=AsyncMock()):
            async with app.run_test() as pilot:
                panel = app.query(ServerPanel).first()
                app._mode[panel] = "docker"
                app._frames[panel] = ["line1", "line2"]
                await app.action_switch_stats()
                assert "line1" in panel.output.content

    # --- action_run_upgrade ---

    async def test_run_upgrade_shows_message_when_no_cmd(self):
        servers = [{"host": "a"}]
        app = MonitorApp(servers)
        with patch.object(MonitorApp, "on_mount", new=AsyncMock()):
            async with app.run_test() as pilot:
                panel = app.query(ServerPanel).first()
                await pilot.pause()
                task = asyncio.create_task(app._run_upgrade(panel, servers[0]))
                app._tasks[panel] = task
                await pilot.pause()
                assert "No upgrade_cmd" in panel.output.content

    async def test_run_upgrade_runs_ssh_and_shows_output(self):
        servers = [{"host": "a", "upgrade_cmd": "apt upgrade -y"}]
        app = MonitorApp(servers)
        proc = make_proc(b"upgrade output\n", b"")
        with (
            patch.object(MonitorApp, "on_mount", new=AsyncMock()),
            patch("asyncio.create_subprocess_exec",
                  new_callable=AsyncMock, return_value=proc),
        ):
            async with app.run_test() as pilot:
                panel = app.query(ServerPanel).first()
                await pilot.pause()
                task = asyncio.create_task(app._run_upgrade(panel, servers[0]))
                app._tasks[panel] = task
                await pilot.pause()
                assert "Upgrade on a" in panel.output.content
                assert "upgrade output" in panel.output.content

    # --- gen-based stale guard ---

    def test_current_gen_mismatch_skips_restart(self):
        app = MonitorApp([{"host": "a"}])
        panel = MagicMock()
        app._gen[panel] = 99
        assert app._current_gen(panel) != 1

    def test_current_gen_match_allows_restart(self):
        app = MonitorApp([{"host": "a"}])
        panel = MagicMock()
        app._gen[panel] = 1
        assert app._current_gen(panel) == 1

    # --- _run_ssh_cmd gen guard on update ---

    async def test_run_ssh_cmd_stale_gen_skips_success_update(self):
        servers = [{"host": "a"}]
        app = MonitorApp(servers)
        async with app.run_test() as pilot:
            panel = app.query(ServerPanel).first()
            proc = make_proc(b"result\n", b"")
            with (
                patch.object(MonitorApp, "on_mount", new=AsyncMock()),
                patch("asyncio.create_subprocess_exec",
                      new_callable=AsyncMock, return_value=proc),
            ):
                await pilot.pause()
                app._gen[panel] = 99
                await app._run_ssh_cmd(panel, servers[0], "some cmd", 1, "TestLabel")
                await pilot.pause()
                assert "TestLabel" not in str(panel.output.content)

    async def test_run_ssh_cmd_stale_gen_skips_error_update(self):
        servers = [{"host": "a"}]
        app = MonitorApp(servers)
        async with app.run_test() as pilot:
            panel = app.query(ServerPanel).first()
            with (
                patch.object(MonitorApp, "on_mount", new=AsyncMock()),
                patch("asyncio.create_subprocess_exec",
                      side_effect=RuntimeError("boom")),
            ):
                await pilot.pause()
                app._gen[panel] = 99
                await app._run_ssh_cmd(panel, servers[0], "x", 1, "TestLabel")
                await pilot.pause()
                assert "boom" not in str(panel.output.content)
                assert "TestLabel" not in str(panel.output.content)

    # --- _monitor_server proc cleanup ---

    async def test_monitor_server_cleans_proc_on_exit(self):
        servers = [{"host": "x"}]
        app = MonitorApp(servers)
        async with app.run_test() as pilot:
            panel = app.query(ServerPanel).first()
            proc = make_proc(b"")
            with (
                patch.object(MonitorApp, "on_mount", new=AsyncMock()),
                patch("asyncio.create_subprocess_exec",
                      new_callable=AsyncMock, return_value=proc),
            ):
                await pilot.pause()
                app._tasks[panel] = asyncio.create_task(
                    app._monitor_server(panel, servers[0])
                )
                await pilot.pause()
                assert panel not in app._procs


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