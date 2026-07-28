import os
import re
import subprocess
import sys

import pytest

from monitor import (
    MonitorError,
    build_ssh_cmd,
    get_ssh_target,
    parse_toml_servers,
    require_commands,
    validate_user,
    kill_existing_session,
    teardown_session,
    create_session,
    bind_exit_keys,
    main,
    SESSION_NAME,
)


class TestBuildSshCmd:
    def test_basic(self):
        result = build_ssh_cmd("192.168.0.33", 22)
        assert result[:5] == ["ssh", "-t", "192.168.0.33", "-p", "22"]
        assert result[5].startswith("python3 -c ")
        assert "read_proc" in result[5] or "get_cpu" in result[5]

    def test_custom_port(self):
        result = build_ssh_cmd("192.168.0.90", 2222)
        assert result[:5] == ["ssh", "-t", "192.168.0.90", "-p", "2222"]
        assert result[5].startswith("python3 -c ")

    def test_with_user(self):
        result = build_ssh_cmd("admin@192.168.0.33", 22)
        assert result[:5] == ["ssh", "-t", "admin@192.168.0.33", "-p", "22"]
        assert result[5].startswith("python3 -c ")

    def test_with_display_ip(self):
        result = build_ssh_cmd("192.168.0.33", 22, display_ip="192.168.0.33")
        assert result[5].count(" ") >= 2
        assert "192.168.0.33" in result[5].split()[-1]


class TestGetSshTarget:
    def test_no_user_default_port(self):
        target, port = get_ssh_target({"host": "192.168.0.33"})
        assert target == "192.168.0.33"
        assert port == 22

    def test_with_user(self):
        target, port = get_ssh_target({"host": "192.168.0.90", "user": "admin"})
        assert target == "admin@192.168.0.90"
        assert port == 22

    def test_custom_port(self):
        target, port = get_ssh_target({"host": "192.168.0.158", "port": 2222})
        assert target == "192.168.0.158"
        assert port == 2222

    def test_with_user_and_custom_port(self):
        target, port = get_ssh_target({"host": "192.168.0.33", "user": "root", "port": 2222})
        assert target == "root@192.168.0.33"
        assert port == 2222


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


class TestKillExistingSession:
    def test_no_session(self, monkeypatch):
        def fake_run(*args, **kw):
            return subprocess.CompletedProcess(args, 1)

        monkeypatch.setattr("subprocess.run", fake_run)
        kill_existing_session()

    def test_kills_session(self, monkeypatch, capsys):
        calls = []

        def fake_run(cmd, *a, **kw):
            calls.append(cmd)
            if "has-session" in cmd:
                return subprocess.CompletedProcess(cmd, 0)
            return subprocess.CompletedProcess(cmd, 0)

        monkeypatch.setattr("subprocess.run", fake_run)
        kill_existing_session()
        assert any("has-session" in c for c in calls)
        assert any("kill-session" in c for c in calls)
        captured = capsys.readouterr()
        assert "Killing" in captured.err

    def test_kill_failure_logs_error(self, monkeypatch, capsys):
        calls = []

        def fake_run(cmd, *a, **kw):
            calls.append(cmd)
            if "has-session" in cmd:
                return subprocess.CompletedProcess(cmd, 0)
            raise subprocess.CalledProcessError(1, cmd)

        monkeypatch.setattr("subprocess.run", fake_run)
        kill_existing_session()
        captured = capsys.readouterr()
        assert "Failed to kill" in captured.err


class TestTeardownSession:
    def test_runs_unbind_and_set(self, monkeypatch):
        calls = []

        def fake_run(cmd, *a, **kw):
            calls.append(cmd)

        monkeypatch.setattr("subprocess.run", fake_run)
        teardown_session()
        assert any("unbind-key" in c for c in calls)
        assert any("status-left" in c for c in calls)


class TestCreateSession:
    def test_single_server(self, monkeypatch):
        calls = []

        def fake_run(cmd, *a, **kw):
            calls.append(cmd)
            return subprocess.CompletedProcess(cmd, 0)

        monkeypatch.setattr("subprocess.run", fake_run)
        create_session([{"host": "192.168.0.33"}])
        assert any("new-session" in c for c in calls)
        assert any("select-layout" in c for c in calls)
        assert not any("split-window" in c for c in calls)
        ssh_cmd = next(c for c in calls if "new-session" in c)
        assert "192.168.0.33" in ssh_cmd[-1]

    def test_multi_server(self, monkeypatch):
        calls = []

        def fake_run(cmd, *a, **kw):
            calls.append(cmd)
            return subprocess.CompletedProcess(cmd, 0)

        monkeypatch.setattr("subprocess.run", fake_run)
        servers = [
            {"host": "192.168.0.33"},
            {"host": "192.168.0.90"},
            {"host": "192.168.0.158"},
        ]
        create_session(servers)
        assert any("new-session" in c for c in calls)
        assert any("split-window" in c for c in calls)
        assert any("select-layout" in c for c in calls)
        split_calls = [c for c in calls if "split-window" in c]
        assert len(split_calls) == 2
        for c in calls:
            if any(x in c for x in ("new-session", "split-window")):
                assert any(ip in c[-1] for ip in ("33", "90", "158"))

    def test_new_session_failure_raises(self, monkeypatch):
        def fake_run(cmd, *a, **kw):
            if "new-session" in cmd:
                raise subprocess.CalledProcessError(1, cmd)
            return subprocess.CompletedProcess(cmd, 0)

        monkeypatch.setattr("subprocess.run", fake_run)
        with pytest.raises(subprocess.CalledProcessError):
            create_session([{"host": "192.168.0.33"}])


class TestBindExitKeys:
    def test_binds_keys(self, monkeypatch):
        calls = []

        def fake_run(cmd, *a, **kw):
            calls.append(cmd)
            return subprocess.CompletedProcess(cmd, 0)

        monkeypatch.setattr("subprocess.run", fake_run)
        bind_exit_keys()
        assert any("bind-key" in c for c in calls)
        assert any("Escape" in c for c in calls)
        assert any("status-left" in c for c in calls)
        assert any("status-left-length" in c for c in calls)

    def test_bind_failure_raises(self, monkeypatch):
        def fake_run(cmd, *a, **kw):
            raise subprocess.CalledProcessError(1, cmd)

        monkeypatch.setattr("subprocess.run", fake_run)
        with pytest.raises(subprocess.CalledProcessError):
            bind_exit_keys()


class TestMain:
    def test_missing_config_exits(self, monkeypatch, tmp_path):
        config_path = str(tmp_path / "nonexistent.toml")
        monkeypatch.setattr("monitor.CONFIG_PATH", config_path)
        with pytest.raises(SystemExit):
            main()

    def test_missing_command_exits(self, monkeypatch, tmp_path):
        p = tmp_path / "config.toml"
        p.write_text("[[servers]]\nhost = '192.168.0.33'\n")
        monkeypatch.setattr("monitor.CONFIG_PATH", str(p))
        monkeypatch.setattr("shutil.which", lambda c: None)
        with pytest.raises(SystemExit):
            main()

    def test_full_lifecycle(self, monkeypatch, tmp_path, capsys):
        p = tmp_path / "config.toml"
        p.write_text("[[servers]]\nhost = '192.168.0.33'\n")
        monkeypatch.setattr("monitor.CONFIG_PATH", str(p))

        calls = []

        def fake_subprocess(cmd, *a, **kw):
            calls.append(cmd)
            if "has-session" in cmd:
                return subprocess.CompletedProcess(cmd, 1)
            if "attach-session" in cmd:
                raise subprocess.CalledProcessError(1, cmd)
            return subprocess.CompletedProcess(cmd, 0)

        monkeypatch.setattr("subprocess.run", fake_subprocess)
        main()
        captured = capsys.readouterr()
        assert "Monitoring" in captured.err or "Attach failed" in captured.err
        assert len(calls) > 0

    def test_setup_failure_cleans_up(self, monkeypatch, tmp_path, capsys):
        p = tmp_path / "config.toml"
        p.write_text("[[servers]]\nhost = '192.168.0.33'\n")
        monkeypatch.setattr("monitor.CONFIG_PATH", str(p))

        def fake_subprocess(cmd, *a, **kw):
            if "has-session" in cmd:
                return subprocess.CompletedProcess(cmd, 1)
            if "bind-key" in cmd:
                raise subprocess.CalledProcessError(1, cmd)
            return subprocess.CompletedProcess(cmd, 0)

        monkeypatch.setattr("subprocess.run", fake_subprocess)
        with pytest.raises(SystemExit):
            main()
        captured = capsys.readouterr()
        assert "Setup failed" in captured.err or "Error" in captured.err


class TestIntegration:
    def test_full_config_parse(self):
        path = os.path.expanduser("~/.config/monitor/config.toml")
        if not os.path.exists(path):
            pytest.skip("No real config file available at ~/.config/monitor/config.toml")
        result = parse_toml_servers(path)
        assert len(result) >= 1
        for s in result:
            assert "host" in s