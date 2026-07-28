import os
import sys

import pytest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))
from compact_monitor import (
    Colors,
    _iteration,
    _render_output,
    _set_ansi,
    fmt_size,
    fmt_rate,
    core_bar,
    get_top_procs,
    make_bar,
    parse_proc_stat,
)

_set_ansi(True)


class TestFmtSize:
    def test_bytes(self):
        assert fmt_size(512) == "512B"

    def test_kib(self):
        assert fmt_size(1024) == "1.0KiB"

    def test_mib(self):
        assert fmt_size(2 * 1024 * 1024) == "2.0MiB"

    def test_gib(self):
        assert fmt_size(1024 ** 3) == "1.0GiB"

    def test_tib(self):
        assert fmt_size(1024 ** 4) == "1.0TiB"

    def test_edge_ki(self):
        assert fmt_size(1023) == "1023B"

    def test_edge_mi(self):
        assert fmt_size(1024 * 1024 - 1) == "1024.0KiB"

    def test_rounding(self):
        result = fmt_size(3 * 1024 * 1024 * 1024)
        assert "GiB" in result


class TestFmtRate:
    def test_bytes(self):
        assert fmt_rate(500) == "500"

    def test_k(self):
        assert fmt_rate(1500) == "1.5K"

    def test_m(self):
        assert fmt_rate(2 * 1024 * 1024) == "2.0M"

    def test_edge(self):
        assert fmt_rate(1023) == "1023"


class TestParseProcStat:
    def test_empty(self):
        agg, cores = parse_proc_stat("")
        assert agg == (0, 0)
        assert cores == {}

    def test_aggregate_only(self):
        data = "cpu  100 20 30 40 50 60 70 80 90 100"
        agg, cores = parse_proc_stat(data)
        assert agg[0] > 0
        assert agg[1] > 0

    def test_with_cores(self):
        data = (
            "cpu  100 20 30 40 50 60 70\n"
            "cpu0 80 10 20 30 0 0 0\n"
            "cpu1 60 15 10 20 0 0 0\n"
        )
        agg, cores = parse_proc_stat(data)
        assert 0 in cores
        assert 1 in cores
        assert cores[0][0] > 0
        assert cores[0][1] > 0
        assert cores[1][0] > 0
        assert cores[1][1] > 0
        assert "intr" not in str(cores)
        assert len(cores) == 2

    def test_non_cpu_lines_ignored(self):
        data = (
            "cpu  100 20 30 40 50 60 70 80 90 100\n"
            "intr 100 200 300\n"
            "ctxt 5000\n"
        )
        agg, cores = parse_proc_stat(data)
        assert cores == {}
        assert agg[0] > 0

    def test_cpu_pct_calculation(self):
        prev_total = 1000
        prev_idle = 800
        curr_total = 2000
        curr_idle = 1400
        delta_total = curr_total - prev_total
        delta_idle = curr_idle - prev_idle
        pct = (delta_total - delta_idle) / delta_total * 100
        assert abs(pct - 40.0) < 0.01


class TestColors:
    def test_cpu_bar_green_low(self):
        assert Colors.cpu_bar(10) == Colors.GREEN

    def test_cpu_bar_yellow_mid(self):
        assert Colors.cpu_bar(60) == Colors.YELLOW

    def test_cpu_bar_red_high(self):
        assert Colors.cpu_bar(85) == Colors.RED

    def test_cpu_bar_red_boundary(self):
        assert Colors.cpu_bar(80) == Colors.RED

    def test_cpu_bar_yellow_boundary(self):
        assert Colors.cpu_bar(50) == Colors.YELLOW

    def test_mem_bar_cyan_low(self):
        assert Colors.mem_bar(10) == Colors.CYAN

    def test_mem_bar_yellow_mid(self):
        assert Colors.mem_bar(60) == Colors.YELLOW

    def test_mem_bar_red_high(self):
        assert Colors.mem_bar(85) == Colors.RED

    def test_mem_bar_cyan_below_50(self):
        assert Colors.mem_bar(49) == Colors.CYAN

    def test_disk_bar_green_low(self):
        assert Colors.disk_bar(10) == Colors.GREEN

    def test_disk_bar_yellow_mid(self):
        assert Colors.disk_bar(75) == Colors.YELLOW

    def test_disk_bar_red_high(self):
        assert Colors.disk_bar(95) == Colors.RED

    def test_disk_bar_green_boundary(self):
        assert Colors.disk_bar(69) == Colors.GREEN

    def test_disk_bar_yellow_boundary(self):
        assert Colors.disk_bar(70) == Colors.YELLOW


class TestMakeBar:
    def test_basic(self):
        bar = make_bar(50, 10, Colors.GREEN)
        assert bar.startswith(Colors.GREEN)
        assert bar.endswith(Colors.RESET)
        assert "#" in bar
        assert "." in bar

    def test_zero_pct(self):
        bar = make_bar(0, 10, Colors.GREEN)
        assert bar.count(".") == 10
        assert bar.count("#") == 0

    def test_full_pct(self):
        bar = make_bar(100, 10, Colors.GREEN)
        assert bar.count("#") == 10
        assert bar.count(".") == 0

    def test_bar_length(self):
        bar = make_bar(50, 8, Colors.GREEN)
        assert len(bar) == 8 + len(Colors.GREEN) + len(Colors.RESET) + 2

    def test_rounding(self):
        bar = make_bar(55, 10, Colors.GREEN)
        assert bar.count("#") == 5


class TestReadProc:
    def test_read_proc_exists(self):
        from compact_monitor import read_proc
        result = read_proc("/dev/null")
        assert result == ""

    def test_read_proc_missing(self):
        from compact_monitor import read_proc
        result = read_proc("/nonexistent/path")
        assert result == ""


class TestGetMemory:
    def test_with_data(self, monkeypatch):
        monkeypatch.setattr("compact_monitor.read_proc", lambda p: (
            "MemTotal: 8000000 kB\n"
            "MemFree: 2000000 kB\n"
            "Buffers: 500000 kB\n"
            "Cached: 3000000 kB\n"
        ))
        from compact_monitor import get_memory
        total, used, pct = get_memory()
        assert total > 0
        assert used > 0
        assert pct > 0

    def test_empty_data(self, monkeypatch):
        monkeypatch.setattr("compact_monitor.read_proc", lambda p: "")
        from compact_monitor import get_memory
        total, used, pct = get_memory()
        assert total == 0
        assert used == 0
        assert pct == 0.0

    def test_skip_line_without_colon(self, monkeypatch):
        monkeypatch.setattr("compact_monitor.read_proc", lambda p: "MemTotal: 4000000 kB\nsome junk line\nMemFree: 2000000 kB\n")
        from compact_monitor import get_memory
        total, used, pct = get_memory()
        assert total > 0


class TestGetDisk:
    def test_no_mountinfo(self, monkeypatch):
        monkeypatch.setattr("compact_monitor.read_proc", lambda p: "")
        from compact_monitor import get_disk
        total, used, pct = get_disk()
        assert total == 0
        assert used == 0
        assert pct == 0.0

    def test_no_root_mount(self, monkeypatch):
        monkeypatch.setattr("compact_monitor.read_proc", lambda p: (
            "1 2 3 4 /subdir - ext4 /dev/sda1 rw,relatime\n"
        ))
        from compact_monitor import get_disk
        total, used, pct = get_disk()
        assert total == 0
        assert used == 0
        assert pct == 0.0

    def test_with_root_mount(self, monkeypatch):
        def fake_read(path):
            if "mountinfo" in path:
                return "1 2 3 4 / - ext4 /dev/sda1 rw,relatime\n"
            return ""
        monkeypatch.setattr("compact_monitor.read_proc", fake_read)
        monkeypatch.setattr("os.statvfs", lambda p: type("st", (), {
            "f_blocks": 1000000,
            "f_bavail": 100000,
            "f_frsize": 4096,
        })())
        from compact_monitor import get_disk
        total, used, pct = get_disk()
        assert total > 0
        assert pct > 0


class TestGetNet:
    def test_no_data(self, monkeypatch):
        monkeypatch.setattr("compact_monitor.read_proc", lambda p: "")
        from compact_monitor import get_net
        rx, tx = get_net()
        assert rx == 0
        assert tx == 0

    def test_with_interfaces(self, monkeypatch):
        monkeypatch.setattr("compact_monitor.read_proc", lambda p: (
            "Inter-|   Receive\n"
            " face |bytes    packets\n"
            "eth0: 1000000 1000 0 0 0 0 0 0 2000000 2000\n"
            "  lo: 500000 500 0 0 0 0 0 0 500000 500\n"
        ))
        from compact_monitor import get_net
        rx, tx = get_net()
        assert rx == 1000000
        assert tx == 2000000

    def test_ignores_loopback(self, monkeypatch):
        monkeypatch.setattr("compact_monitor.read_proc", lambda p: (
            "Inter-|   Receive\n"
            " face |bytes    packets\n"
            "  lo: 999999 500 0 0 0 0 0 0 999999 500\n"
        ))
        from compact_monitor import get_net
        rx, tx = get_net()
        assert rx == 0
        assert tx == 0


class TestGetTopProcs:
    def test_no_ps_output(self, monkeypatch):
        monkeypatch.setattr(
            "compact_monitor.subprocess.run",
            lambda *a, **kw: type("r", (), {"stdout": ""})(),
        )
        from compact_monitor import get_top_procs
        assert get_top_procs(5) == []

    def test_ps_command_not_found(self, monkeypatch):
        monkeypatch.setattr(
            "compact_monitor.subprocess.run",
            lambda *a, **kw: (_ for _ in ()).throw(FileNotFoundError),
        )
        from compact_monitor import get_top_procs
        assert get_top_procs() == []


class TestGetHostInfo:
    def test_with_display_ip(self, monkeypatch):
        monkeypatch.setattr("sys.argv", ["script", "192.168.0.33"])
        monkeypatch.setattr("socket.gethostname", lambda: "myserver")
        from compact_monitor import get_host_info
        result = get_host_info()
        assert "myserver" in result
        assert "192.168.0.33" in result

    def test_with_proc_hostname(self, monkeypatch):
        monkeypatch.setattr("sys.argv", ["script", "10.0.0.1"])
        monkeypatch.setattr("socket.gethostname", lambda: "wrong")
        monkeypatch.setattr("compact_monitor.read_proc", lambda p: "prochost\n" if "hostname" in p else "")
        from compact_monitor import get_host_info
        result = get_host_info()
        assert "prochost" in result

    def test_auto_ip_via_fib_trie(self, monkeypatch):
        monkeypatch.setattr("sys.argv", ["script"])
        monkeypatch.setattr("socket.gethostname", lambda: "srv")
        def fake_ip(*a, **kw):
            raise FileNotFoundError
        monkeypatch.setattr("compact_monitor.subprocess.run", fake_ip)
        monkeypatch.setattr("compact_monitor.read_proc", lambda p: (
            "src 192.168.1.100\n" if "fib_trie" in p else ""
        ))
        from compact_monitor import get_host_info
        result = get_host_info()
        assert "192.168.1.100" in result

    def test_auto_ip_fallback(self, monkeypatch):
        monkeypatch.setattr("sys.argv", ["script"])
        monkeypatch.setattr("socket.gethostname", lambda: "srv")
        def fake_ip(*a, **kw):
            raise FileNotFoundError
        monkeypatch.setattr("compact_monitor.subprocess.run", fake_ip)
        monkeypatch.setattr("compact_monitor.read_proc", lambda p: "")
        from compact_monitor import get_host_info
        result = get_host_info()
        assert "srv" in result

    def test_ip_command_success(self, monkeypatch):
        monkeypatch.setattr("sys.argv", ["script"])
        monkeypatch.setattr("socket.gethostname", lambda: "srv")
        def fake_run(*a, **kw):
            return type("r", (), {"stdout": "1: eth0    inet 10.0.0.5/24 scope global eth0\n"})()
        monkeypatch.setattr("compact_monitor.subprocess.run", fake_run)
        from compact_monitor import get_host_info
        result = get_host_info()
        assert "10.0.0.5" in result

    def test_ip_malformed_line(self, monkeypatch):
        monkeypatch.setattr("sys.argv", ["script"])
        monkeypatch.setattr("socket.gethostname", lambda: "srv")
        def fake_run(*a, **kw):
            return type("r", (), {"stdout": "short\n"})()
        monkeypatch.setattr("compact_monitor.subprocess.run", fake_run)
        monkeypatch.setattr("compact_monitor.read_proc", lambda p: "")
        from compact_monitor import get_host_info
        result = get_host_info()
        assert result == "srv"

    def test_fib_trie_ipv6(self, monkeypatch):
        monkeypatch.setattr("sys.argv", ["script"])
        monkeypatch.setattr("socket.gethostname", lambda: "srv")
        def fake_run(*a, **kw):
            raise FileNotFoundError
        monkeypatch.setattr("compact_monitor.subprocess.run", fake_run)
        monkeypatch.setattr("compact_monitor.read_proc", lambda p: (
            "src fe80::1\n"
            "src 10.0.0.1\n"
            if "fib_trie" in p else ""
        ))
        from compact_monitor import get_host_info
        result = get_host_info()
        assert "10.0.0.1" in result
        assert "fe80" not in result

    def test_hostname_from_proc_no_argv(self, monkeypatch):
        monkeypatch.setattr("sys.argv", ["script"])
        monkeypatch.setattr("socket.gethostname", lambda: "srv")
        def fake_run(*a, **kw):
            raise FileNotFoundError
        monkeypatch.setattr("compact_monitor.subprocess.run", fake_run)
        monkeypatch.setattr("compact_monitor.read_proc", lambda p: (
            "unrelated line\n"
            "src 192.168.1.1\n"
            if "fib_trie" in p else ""
        ))
        from compact_monitor import get_host_info
        result = get_host_info()
        assert "192.168.1.1" in result

    def test_ip_success_no_match(self, monkeypatch):
        monkeypatch.setattr("sys.argv", ["script"])
        monkeypatch.setattr("socket.gethostname", lambda: "srv")
        def fake_run(*a, **kw):
            return type("r", (), {"stdout": ""})()
        monkeypatch.setattr("compact_monitor.subprocess.run", fake_run)
        monkeypatch.setattr("compact_monitor.read_proc", lambda p: "")
        from compact_monitor import get_host_info
        result = get_host_info()
        assert result == "srv"


class TestGetTopProcsEdgeCases:
    def test_malformed_ps_line(self, monkeypatch):
        monkeypatch.setattr(
            "compact_monitor.subprocess.run",
            lambda *a, **kw: type("r", (), {"stdout": "1234  0.0\n"})(),
        )
        from compact_monitor import get_top_procs
        procs = get_top_procs(5)
        assert len(procs) == 0

    def test_incomplete_line(self, monkeypatch):
        monkeypatch.setattr(
            "compact_monitor.subprocess.run",
            lambda *a, **kw: type("r", (), {"stdout": "  \n1234  0.0  1024  python3\n"})(),
        )
        from compact_monitor import get_top_procs
        procs = get_top_procs(5)
        assert len(procs) == 1
        assert procs[0][1] == "python3"

    def test_empty_line(self, monkeypatch):
        monkeypatch.setattr(
            "compact_monitor.subprocess.run",
            lambda *a, **kw: type("r", (), {"stdout": "1234  0.0  1024  python3\n  \n5678  3.5  2048  bash\n"})(),
        )
        from compact_monitor import get_top_procs
        procs = get_top_procs(5)
        assert len(procs) == 2
        assert procs[0][1] == "python3"
        assert procs[1][1] == "bash"

    def test_line_without_name(self, monkeypatch):
        monkeypatch.setattr(
            "compact_monitor.subprocess.run",
            lambda *a, **kw: type("r", (), {"stdout": "5678  3.5  2048\n"})(),
        )
        from compact_monitor import get_top_procs
        procs = get_top_procs(5)
        assert len(procs) == 1
        assert procs[0][0] == "5678"
        assert procs[0][1] == "?"
        assert procs[0][2] == "3.5"


class TestCoreBar:
    def test_basic(self):
        bar = core_bar(50, 10)
        assert "#" in bar
        assert "." in bar

    def test_zero_pct(self):
        bar = core_bar(0, 10)
        assert bar.count(".") == 10

    def test_full_pct(self):
        bar = core_bar(100, 10)
        assert bar.count("#") == 10

    def test_color_green(self):
        bar = core_bar(10, 10)
        assert bar.startswith(Colors.GREEN)

    def test_color_yellow(self):
        bar = core_bar(60, 10)
        assert bar.startswith(Colors.YELLOW)

    def test_color_red(self):
        bar = core_bar(90, 10)
        assert bar.startswith(Colors.RED)

    def test_length(self):
        bar = core_bar(50, 8)
        assert len(bar) == 8 + len(Colors.RESET) + len(Colors.cpu_bar(50))


class TestRenderOutput:
    def make_host(self, cols=80):
        return "testhost (192.168.0.1)"

    def test_host_line(self):
        out = _render_output("h", 80, 50, 50, [], 0, 0, 0, 0, 0, 0, 0, 0, [])
        assert any("h" in l for l in out)

    def test_single_core_uses_aggregate_bar(self):
        out = _render_output("h", 80, 50, 42, [],
                             2**31, 2**30, 50.0,
                             2**40, 2**38, 80.0,
                             0, 0, [])
        cpu_line = out[1]
        assert "CPU" in cpu_line
        assert "42%" in cpu_line
        assert "[" in cpu_line

    def test_dual_core_shows_per_core(self):
        out = _render_output("h", 80, 50, 50, [(0, 75), (1, 25)],
                             2**31, 2**30, 50.0,
                             2**40, 2**38, 80.0,
                             0, 0, [])
        cpu_line = out[1]
        assert "CPU" in cpu_line
        assert "0:" in cpu_line
        assert "1:" in cpu_line
        assert "75%" in cpu_line
        assert "25%" in cpu_line

    def test_many_cores_wraps_lines(self):
        core_lines = [(i, float(i * 10)) for i in range(8)]
        out = _render_output("h", 40, 20, 50, core_lines,
                             2**31, 2**30, 50.0,
                             2**40, 2**38, 80.0,
                             0, 0, [])
        cpu_lines = [l for l in out if "CPU" in l or l.strip().startswith(tuple(str(i) for i in range(8)))]
        assert len(cpu_lines) >= 2

    def test_mem_shown_when_total_given(self):
        out = _render_output("h", 80, 50, 50, [],
                             2**31, 2**30, 50.0,
                             0, 0, 0,
                             0, 0, [])
        mem_lines = [l for l in out if Colors.BOLD + "MEM" + Colors.RESET in l]
        assert len(mem_lines) == 1
        assert "50%" in mem_lines[0]
        assert "GiB" in mem_lines[0]

    def test_mem_omitted_when_zero(self):
        out = _render_output("h", 80, 50, 50, [],
                             0, 0, 0,
                             2**40, 2**38, 80.0,
                             0, 0, [])
        mem_lines = [l for l in out if Colors.BOLD + "MEM" + Colors.RESET in l]
        assert len(mem_lines) == 0

    def test_disk_shown_when_total_given(self):
        out = _render_output("h", 80, 50, 50, [],
                             2**31, 2**30, 50.0,
                             2**40, 2**38, 80.0,
                             0, 0, [])
        dsk_lines = [l for l in out if Colors.BOLD + "DSK" + Colors.RESET in l]
        assert len(dsk_lines) == 1
        assert "80%" in dsk_lines[0]
        assert "TiB" in dsk_lines[0]

    def test_disk_omitted_when_zero(self):
        out = _render_output("h", 80, 50, 50, [],
                             2**31, 2**30, 50.0,
                             0, 0, 0,
                             0, 0, [])
        dsk_lines = [l for l in out if Colors.BOLD + "DSK" + Colors.RESET in l]
        assert len(dsk_lines) == 0

    def test_net_shown_when_traffic(self):
        out = _render_output("h", 80, 50, 50, [],
                             0, 0, 0,
                             0, 0, 0,
                             2000000, 3000000, [])
        net_lines = [l for l in out if "NET" in l]
        assert len(net_lines) == 1
        assert "\u2191" in net_lines[0] or "^" in net_lines[0]

    def test_net_omitted_when_low(self):
        out = _render_output("h", 80, 50, 50, [],
                             0, 0, 0,
                             0, 0, 0,
                             500, 500, [])
        net_lines = [l for l in out if "NET" in l]
        assert len(net_lines) == 0

    def test_procs_appended(self):
        procs = [("100", "python3", "2.5", 45000),
                 ("200", "bash", "0.5", 12000)]
        out = _render_output("h", 80, 50, 50, [],
                             2**31, 2**30, 50.0,
                             2**40, 2**38, 80.0,
                             0, 0, procs)
        assert any("python3" in l for l in out)
        assert any("bash" in l for l in out)
        assert any("PID" in l for l in out)

    def test_proc_name_truncation(self):
        procs = [("1", "verylongprocessnameishere", "1.0", 1000)]
        out = _render_output("h", 80, 50, 50, [],
                             2**31, 2**30, 50.0,
                             2**40, 2**38, 80.0,
                             0, 0, procs)
        assert any("..." in l for l in out)

    def test_high_cpu_proc_highlighted(self):
        procs = [("1", "hungry", "95.0", 1000)]
        out = _render_output("h", 80, 50, 50, [],
                             2**31, 2**30, 50.0,
                             2**40, 2**38, 80.0,
                             0, 0, procs)
        cpu_line = next(l for l in out if "hungry" in l)
        assert Colors.YELLOW in cpu_line

    def test_low_cpu_proc_not_highlighted(self):
        procs = [("1", "idle", "1.0", 1000)]
        out = _render_output("h", 80, 50, 50, [],
                             2**31, 2**30, 50.0,
                             2**40, 2**38, 80.0,
                             0, 0, procs)
        cpu_line = next(l for l in out if "idle" in l)
        assert Colors.YELLOW not in cpu_line

    def test_full_layout(self):
        procs = [("1", "top", "10.0", 2**20)]
        out = _render_output("h", 80, 50, 30, [(0, 60), (1, 20)],
                             2**31, 2**30, 50.0,
                             2**40, 2**38, 80.0,
                             5000, 15000, procs)
        assert len(out) >= 6
        assert all(isinstance(l, str) for l in out)


class TestSetAnsi:
    def test_enable(self):
        Colors.RESET = ""
        _set_ansi(True)
        assert Colors._ansi
        assert Colors.RESET == "\033[0m"

    def test_disable(self):
        _set_ansi(True)
        _set_ansi(False)
        assert not Colors._ansi
        assert Colors.RESET == ""


class TestIteration:
    def test_with_mocked_io(self, monkeypatch):
        monkeypatch.setattr("compact_monitor.read_proc", lambda p: "")
        monkeypatch.setattr("compact_monitor.get_net", lambda: (0, 0))
        monkeypatch.setattr("compact_monitor.get_top_procs", lambda n: [("1", "init", "0.0", 1000)])
        monkeypatch.setattr("compact_monitor.get_memory", lambda: (0, 0, 0.0))
        monkeypatch.setattr("compact_monitor.get_disk", lambda: (0, 0, 0.0))
        out, agg, cores, net = _iteration(
            "test", (0, 0), {}, (0, 0), 2, 80, 24,
        )
        assert len(out) > 0
        assert any("test" in l for l in out)
        assert any("init" in l for l in out)

    def test_with_mem_disk_net(self, monkeypatch):
        monkeypatch.setattr("compact_monitor.read_proc", lambda p: "")
        monkeypatch.setattr("compact_monitor.get_net", lambda: (50000, 30000))
        monkeypatch.setattr("compact_monitor.get_top_procs", lambda n: [])
        monkeypatch.setattr("compact_monitor.get_memory", lambda: (2**31, 2**30, 50.0))
        monkeypatch.setattr("compact_monitor.get_disk", lambda: (2**40, 2**38, 80.0))
        out, agg, cores, net = _iteration(
            "test", (50000, 30000), {}, (0, 0), 2, 80, 24,
        )
        assert any("MEM" in l for l in out)
        assert any("DSK" in l for l in out)
        assert any("NET" in l for l in out)

    def test_with_nonzero_net_rate(self, monkeypatch):
        monkeypatch.setattr("compact_monitor.read_proc", lambda p: "")
        monkeypatch.setattr("compact_monitor.get_net", lambda: (5000000, 3000000))
        monkeypatch.setattr("compact_monitor.get_top_procs", lambda n: [])
        monkeypatch.setattr("compact_monitor.get_memory", lambda: (0, 0, 0.0))
        monkeypatch.setattr("compact_monitor.get_disk", lambda: (0, 0, 0.0))
        out, agg, cores, net = _iteration(
            "test", (5000000, 3000000), {}, (0, 0), 2, 80, 24,
        )
        net_lines = [l for l in out if "NET" in l]
        assert len(net_lines) >= 1

    def test_with_multi_core(self, monkeypatch):
        monkeypatch.setattr("compact_monitor.read_proc", lambda p: "cpu  300 0 100 50 0 0 0\ncpu0 80 0 20 10 0 0 0\ncpu1 120 0 80 30 0 0 0\n")
        monkeypatch.setattr("compact_monitor.get_net", lambda: (0, 0))
        monkeypatch.setattr("compact_monitor.get_top_procs", lambda n: [])
        monkeypatch.setattr("compact_monitor.get_memory", lambda: (0, 0, 0.0))
        monkeypatch.setattr("compact_monitor.get_disk", lambda: (0, 0, 0.0))
        out, agg, cores, net = _iteration(
            "test", (200, 30), {0: (50, 5), 1: (150, 20)}, (0, 0), 2, 80, 24,
        )
        assert len(out) > 1
        assert any("CPU" in l for l in out)


class TestLoop:
    def _run_one_iteration(self, monkeypatch, ansi):
        written = []
        monkeypatch.setattr("compact_monitor.Colors._ansi", ansi)
        monkeypatch.setattr("compact_monitor.shutil.get_terminal_size",
                            lambda *a: type("s", (), {"columns": 80, "lines": 24})())
        monkeypatch.setattr("compact_monitor.read_proc", lambda p: "")
        monkeypatch.setattr("compact_monitor.get_net", lambda: (0, 0))
        monkeypatch.setattr("compact_monitor.get_top_procs", lambda n: [])
        monkeypatch.setattr("compact_monitor.get_memory", lambda: (0, 0, 0.0))
        monkeypatch.setattr("compact_monitor.get_disk", lambda: (0, 0, 0.0))
        monkeypatch.setattr("sys.stdout.write", lambda s: written.append(s))
        monkeypatch.setattr("sys.stdout.flush", lambda: None)
        import compact_monitor
        orig_sleep = compact_monitor.time.sleep
        call_count = 0
        def fake_sleep(s):
            nonlocal call_count
            call_count += 1
            if call_count >= 2:
                raise StopIteration
            orig_sleep(0)
        monkeypatch.setattr("compact_monitor.time.sleep", fake_sleep)
        from compact_monitor import loop
        try:
            loop(0.01, is_tty=ansi)
        except StopIteration:
            pass
        return written

    def test_non_ansi(self, monkeypatch):
        written = self._run_one_iteration(monkeypatch, False)
        assert len(written) >= 1
        assert any("===MONITOR===" in w for w in written)

    def test_ansi(self, monkeypatch):
        written = self._run_one_iteration(monkeypatch, True)
        assert len(written) >= 1
        assert any("\033[H\033[J" in w for w in written)


class TestMain:
    def test_calls_loop(self, monkeypatch):
        calls = []
        monkeypatch.setattr("sys.stdout.isatty", lambda: True)
        monkeypatch.setattr("compact_monitor.loop", lambda i, is_tty=False: calls.append(i))
        from compact_monitor import main
        main()
        assert calls == [2]

    def test_non_tty_no_cursor_hide(self, monkeypatch):
        writes = []
        monkeypatch.setattr("sys.stdout.isatty", lambda: False)
        monkeypatch.setattr("compact_monitor.loop", lambda i, is_tty=False: None)
        monkeypatch.setattr("sys.stdout.write", lambda s: writes.append(s))
        from compact_monitor import main
        main()
        assert not any("?25l" in w for w in writes)