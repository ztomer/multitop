#!/usr/bin/env python3
import os
import re
import shutil
import socket
import subprocess
import sys
import time


def read_proc(path):
    try:
        with open(path) as f:
            return f.read()
    except OSError:
        return ""


class Colors:
    _ansi = False
    RESET = ""
    BOLD = ""
    DIM = ""
    RED = ""
    GREEN = ""
    YELLOW = ""
    BLUE = ""
    CYAN = ""
    WHITE = ""
    GRAY = ""

    @staticmethod
    def cpu_bar(pct):
        if pct >= 80:
            return Colors.RED
        if pct >= 50:
            return Colors.YELLOW
        return Colors.GREEN

    @staticmethod
    def mem_bar(pct):
        if pct >= 80:
            return Colors.RED
        if pct >= 50:
            return Colors.YELLOW
        return Colors.CYAN

    @staticmethod
    def disk_bar(pct):
        if pct >= 90:
            return Colors.RED
        if pct >= 70:
            return Colors.YELLOW
        return Colors.GREEN


def _set_ansi(enabled):
    Colors._ansi = enabled
    if enabled:
        Colors.RESET = "\033[0m"
        Colors.BOLD = "\033[1m"
        Colors.DIM = "\033[2m"
        Colors.RED = "\033[0;31m"
        Colors.GREEN = "\033[0;32m"
        Colors.YELLOW = "\033[0;33m"
        Colors.BLUE = "\033[0;34m"
        Colors.CYAN = "\033[0;36m"
        Colors.WHITE = "\033[0;37m"
        Colors.GRAY = "\033[0;90m"
    else:
        Colors.RESET = Colors.BOLD = Colors.DIM = ""
        Colors.RED = Colors.GREEN = Colors.YELLOW = ""
        Colors.BLUE = Colors.CYAN = Colors.WHITE = Colors.GRAY = ""


def fmt_size(b):
    if b >= 1024 ** 4:
        return f"{b / (1024 ** 4):.1f}TiB"
    if b >= 1024 ** 3:
        return f"{b / (1024 ** 3):.1f}GiB"
    if b >= 1024 ** 2:
        return f"{b / (1024 ** 2):.1f}MiB"
    if b >= 1024:
        return f"{b / 1024:.1f}KiB"
    return f"{int(b)}B"


def fmt_rate(bytes_per_sec):
    if bytes_per_sec >= 1024 * 1024:
        return f"{bytes_per_sec / (1024 * 1024):.1f}M"
    if bytes_per_sec >= 1024:
        return f"{bytes_per_sec / 1024:.1f}K"
    return f"{int(bytes_per_sec)}"


def get_host_info():
    hostname = socket.gethostname()
    data = read_proc("/proc/sys/kernel/hostname").strip()
    if data:
        hostname = data

    if len(sys.argv) > 1:
        return f"{hostname} ({sys.argv[1]})"

    ips = []
    try:
        result = subprocess.run(
            ["ip", "-4", "-o", "addr", "show", "scope", "global"],
            capture_output=True, text=True, timeout=3,
        )
        for line in result.stdout.splitlines():
            parts = line.split()
            if len(parts) >= 4:
                ips.append(parts[3].split("/")[0])
    except (subprocess.SubprocessError, FileNotFoundError):
        pass

    if not ips:
        data = read_proc("/proc/net/fib_trie")
        if data:
            for line in data.splitlines():
                if "src " in line:
                    ip = line.split()[-1]
                    if ":" not in ip:
                        ips.append(ip)

    if ips:
        return f"{hostname} ({ips[0]})"
    return hostname


def parse_proc_stat(data):
    if not data:
        return (0, 0), {}
    lines = data.splitlines()
    first = lines[0].split()
    vals = [int(v) for v in first[1:]]
    agg = (sum(vals), vals[3] + vals[4])
    cores = {}
    for line in lines[1:]:
        if line.startswith("cpu") and line[3].isdigit():
            parts = line.split()
            idx = int(parts[0][3:])
            v = [int(x) for x in parts[1:]]
            cores[idx] = (sum(v), v[3] + v[4])
    return agg, cores


def get_memory():
    data = read_proc("/proc/meminfo")
    if not data:
        return 0, 0, 0.0
    mem = {}
    for line in data.splitlines():
        if ":" in line:
            k, v = line.split(":", 1)
            val = int(v.strip().split()[0])
            mem[k.strip()] = val
    total = mem.get("MemTotal", 0) * 1024
    free = mem.get("MemFree", 0) * 1024
    buffers = mem.get("Buffers", 0) * 1024
    cached = mem.get("Cached", 0) * 1024
    used = total - free - buffers - cached
    pct = (used / total * 100) if total else 0
    return total, used, pct


def get_disk():
    data = read_proc("/proc/self/mountinfo")
    if not data:
        return 0, 0, 0.0
    root_mount = None
    for line in data.splitlines():
        parts = line.split()
        if len(parts) >= 5 and parts[4] == "/":
            root_mount = parts[3]
            break
    if not root_mount:
        return 0, 0, 0.0
    st = os.statvfs(root_mount)
    total = st.f_blocks * st.f_frsize
    free = st.f_bavail * st.f_frsize
    used = total - free
    pct = (used / total * 100) if total else 0
    return total, used, pct


def get_net():
    data = read_proc("/proc/net/dev")
    if not data:
        return 0, 0
    rx_total = 0
    tx_total = 0
    for line in data.splitlines()[2:]:
        parts = line.split()
        iface = parts[0].rstrip(":")
        if iface.startswith("lo"):
            continue
        rx_total += int(parts[1])
        tx_total += int(parts[9])
    return rx_total, tx_total


def get_top_procs(n=5):
    try:
        result = subprocess.run(
            ["ps", "-eo", "pid,pcpu,rss,comm", "--sort=-pcpu", "--no-headers"],
            capture_output=True, text=True, timeout=3,
        )
        lines = result.stdout.strip().splitlines()[:n]
        procs = []
        for line in lines:
            if not line.strip():
                continue
            parts = line.strip().split(None, 3)
            if len(parts) >= 3:
                pid = parts[0]
                cpu = parts[1]
                mem = int(parts[2]) * 1024
                name = parts[3] if len(parts) > 3 else "?"
                procs.append((pid, name, cpu, mem))
        return procs
    except (subprocess.SubprocessError, FileNotFoundError):
        return []


def make_bar(pct, length, color):
    filled = int(pct / 100 * length)
    return f"{color}[{'#' * filled}{'.' * (length - filled)}]{Colors.RESET}"


def core_bar(pct, length):
    filled = int(pct / 100 * length)
    c = Colors.cpu_bar(pct)
    h = "#" * filled
    d = "." * (length - filled)
    return f"{c}{h}{d}{Colors.RESET}"


def _render_output(host, cols, bar_len, cpu_pct, core_lines,
                   mem_total, mem_used, mem_pct,
                   disk_total, disk_used, disk_pct,
                   rx_rate, tx_rate, procs):
    num_cores = len(core_lines)
    out = []

    out.append(
        f"{Colors.CYAN}{Colors.BOLD}{host}{Colors.RESET}"
        f"  {Colors.GRAY}{'─' * max(0, cols - len(host) - 6)}{Colors.RESET}"
    )

    if num_cores >= 2:
        _sa = re.compile(r'\033\[[0-9;]*m').sub
        per_core_bar_len = min(12, bar_len // num_cores)
        show_core_bars = per_core_bar_len >= 5
        segs = []
        for idx, cp in core_lines:
            if show_core_bars:
                segs.append(f"{idx}:{core_bar(cp, per_core_bar_len)}{cp:.0f}%")
            else:
                segs.append(f"{idx}:{cp:.0f}%")
        plain_segs = [_sa("", s) for s in segs]
        cpu_label = f" {Colors.BOLD}CPU{Colors.RESET} "
        cpu_plain = " CPU "
        indent = " " * len(cpu_plain)
        used = 0
        while used < num_cores:
            label = cpu_label if used == 0 else indent
            disp = cpu_plain if used == 0 else indent
            for seg, ps in zip(segs[used:], plain_segs[used:]):
                if len(disp) + len(ps) + 1 > cols:
                    break
                label += seg + " "
                disp += ps + " "
                used += 1
            out.append(label.rstrip())
    else:
        bc = Colors.cpu_bar(cpu_pct)
        out.append(
            f" {Colors.BOLD}CPU{Colors.RESET} {make_bar(cpu_pct, bar_len, bc)} {bc}{cpu_pct:.0f}%{Colors.RESET}"
        )

    if mem_total:
        bc2 = Colors.mem_bar(mem_pct)
        out.append(
            f" {Colors.BOLD}MEM{Colors.RESET}"
            f" {make_bar(mem_pct, bar_len, bc2)} {bc2}{mem_pct:.0f}%{Colors.RESET}"
            f" {Colors.GRAY}{fmt_size(mem_used)}/{fmt_size(mem_total)}{Colors.RESET}"
        )

    if disk_total:
        bc3 = Colors.disk_bar(disk_pct)
        out.append(
            f" {Colors.BOLD}DSK{Colors.RESET}"
            f" {make_bar(disk_pct, bar_len, bc3)} {bc3}{disk_pct:.0f}%{Colors.RESET}"
            f" {Colors.GRAY}{fmt_size(disk_used)}/{fmt_size(disk_total)}{Colors.RESET}"
        )

    show_net = rx_rate > 1024 or tx_rate > 1024
    if show_net:
        out.append(
            f" {Colors.BOLD}NET{Colors.RESET}"
            f" {Colors.GREEN}\u2191 {fmt_rate(tx_rate)}{Colors.RESET}"
            f"  {Colors.CYAN}\u2193 {fmt_rate(rx_rate)}{Colors.RESET}"
        )

    name_w = max(12, min(20, cols - 38))
    out.append(f" {Colors.GRAY}{'─' * max(0, cols - 2)}{Colors.RESET}")
    out.append(
        f" {Colors.BOLD}{'PID':>7}  {'NAME':<{name_w}}  {'CPU':>5}  {'MEM':>7}{Colors.RESET}"
    )
    for pid, name, cpu, mem_bytes in procs:
        name_trunc = name if len(name) < name_w else name[:name_w - 3] + "..."
        cpu_color = Colors.YELLOW if float(cpu or 0) >= 10 else Colors.WHITE
        out.append(
            f" {Colors.GRAY}{pid:>7}{Colors.RESET}"
            f"  {Colors.WHITE}{name_trunc:<{name_w}}{Colors.RESET}"
            f"  {cpu_color}{cpu:>5}{Colors.RESET}"
            f"  {Colors.CYAN}{fmt_size(mem_bytes):>7}{Colors.RESET}"
        )

    return out


def _iteration(host, prev_agg, prev_cores, net_prev, interval, cols, term_lines):
    bar_len = max(8, cols - 16)

    data = read_proc("/proc/stat")
    curr_agg, curr_cores = parse_proc_stat(data)
    net_curr = get_net()
    mem_total, mem_used, mem_pct = get_memory()
    disk_total, disk_used, disk_pct = get_disk()

    t = curr_agg[0] - prev_agg[0]
    i = curr_agg[1] - prev_agg[1]
    cpu_pct = ((t - i) / t * 100) if t else 0

    core_lines = []
    if curr_cores and prev_cores:
        for idx in sorted(curr_cores.keys()):
            tc = curr_cores[idx][0] - prev_cores.get(idx, (0, 0))[0]
            ic = curr_cores[idx][1] - prev_cores.get(idx, (0, 0))[1]
            cp = ((tc - ic) / tc * 100) if tc else 0
            core_lines.append((idx, cp))

    net_rx, net_tx = net_curr
    net_rx_prev, net_tx_prev = net_prev
    rx_rate = (net_rx - net_rx_prev) / interval
    tx_rate = (net_tx - net_tx_prev) / interval

    num_cores = len(core_lines)
    core_rows = ((num_cores + 1) // 2) if num_cores >= 2 else 0
    base = 2 + core_rows
    if mem_total:
        base += 1
    if disk_total:
        base += 1
    if rx_rate > 1024 or tx_rate > 1024:
        base += 1
    max_procs = max(1, term_lines - base - 3)

    procs = get_top_procs(max_procs)

    out = _render_output(host, cols, bar_len, cpu_pct, core_lines,
                         mem_total, mem_used, mem_pct,
                         disk_total, disk_used, disk_pct,
                         rx_rate, tx_rate, procs)

    return out, curr_agg, curr_cores, net_curr


def main():
    _set_ansi(True)
    is_tty = sys.stdout.isatty()
    if is_tty:
        sys.stdout.write("\033[?25l")
    try:
        loop(2, is_tty)
    finally:
        if is_tty:
            sys.stdout.write("\033[?25h")


def loop(interval, is_tty=False):
    host = get_host_info()
    data = read_proc("/proc/stat")
    prev_agg, prev_cores = parse_proc_stat(data)
    net_prev = get_net()

    time.sleep(interval)

    while True:
        term = shutil.get_terminal_size((80, 24))
        cols = term.columns
        term_lines = term.lines

        out, prev_agg, prev_cores, net_prev = _iteration(
            host, prev_agg, prev_cores, net_prev, interval, cols, term_lines,
        )

        if is_tty:
            sys.stdout.write("\033[H\033[J" + "\n".join(out) + "\n")
        else:
            sys.stdout.write("===MONITOR===\n" + "\n".join(out) + "\n")
        sys.stdout.flush()

        time.sleep(interval)


if __name__ == "__main__":  # pragma: no cover
    main()