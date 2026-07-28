#!/usr/bin/env python3
import os
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
    RESET = "\033[0m"
    BOLD = "\033[1m"
    DIM = "\033[2m"
    RED = "\033[0;31m"
    GREEN = "\033[0;32m"
    YELLOW = "\033[0;33m"
    BLUE = "\033[0;34m"
    MAGENTA = "\033[0;35m"
    CYAN = "\033[0;36m"
    WHITE = "\033[0;37m"
    GRAY = "\033[0;90m"

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


def fmt_size(kb):
    if kb >= 1024 * 1024:
        return f"{kb / (1024 * 1024):.1f}TiB"
    if kb >= 1024:
        return f"{kb / 1024:.1f}GiB"
    return f"{kb}MiB"


def fmt_rate(bytes_per_sec):
    if bytes_per_sec >= 1024 * 1024:
        return f"{bytes_per_sec / (1024 * 1024):.1f}M"
    if bytes_per_sec >= 1024:
        return f"{bytes_per_sec / 1024:.1f}K"
    return f"{int(bytes_per_sec)}"


def get_host_info():
    hostname = socket.gethostname()
    try:
        data = read_proc("/proc/sys/kernel/hostname").strip()
        if data:
            hostname = data
    except OSError:
        pass

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
        try:
            data = read_proc("/proc/net/fib_trie")
            if data:
                for line in data.splitlines():
                    if "src " in line:
                        ip = line.split()[-1]
                        if ":" not in ip:
                            ips.append(ip)
        except OSError:
            pass

    if ips:
        return f"{hostname} ({ips[0]})"
    return hostname


def get_cpu():
    prev_total = 0
    prev_idle = 0
    data = read_proc("/proc/stat")
    if not data:
        return 0.0
    first = data.splitlines()[0]
    parts = first.split()
    vals = [int(v) for v in parts[1:]]
    total = sum(vals)
    idle = vals[3] + vals[4]
    return total, idle


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
    total = mem.get("MemTotal", 0)
    free = mem.get("MemFree", 0)
    buffers = mem.get("Buffers", 0)
    cached = mem.get("Cached", 0)
    used = total - free - buffers - cached
    pct = (used / total * 100) if total else 0
    return total // 1024, used // 1024, pct


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
                mem = int(parts[2]) // 1024
                name = parts[3] if len(parts) > 3 else "?"
                procs.append((pid, name, cpu, mem))
        return procs
    except (subprocess.SubprocessError, FileNotFoundError):
        return []


def main():
    sys.stdout.write("\033[?25l")
    try:
        loop(2)
    finally:
        sys.stdout.write("\033[?25h")


def loop(interval):
    host = get_host_info()
    cpu_prev = get_cpu()
    net_prev = get_net()
    time.sleep(interval)

    while True:
        term = shutil.get_terminal_size((80, 24))
        bar_len = max(8, term.columns - 30)

        cpu_curr = get_cpu()
        net_curr = get_net()
        mem_total_kb, mem_used_kb, mem_pct = get_memory()
        disk_total, disk_used, disk_pct = get_disk()

        out = []
        out.append(
            f"{Colors.CYAN}{Colors.BOLD}{host}{Colors.RESET}"
            f"  {Colors.GRAY}{'─' * max(0, term.columns - len(host) - 6)}{Colors.RESET}"
        )

        if isinstance(cpu_curr, tuple) and isinstance(cpu_prev, tuple):
            t = cpu_curr[0] - cpu_prev[0]
            i = cpu_curr[1] - cpu_prev[1]
            cpu_pct = ((t - i) / t * 100) if t else 0
            bc = Colors.cpu_bar(cpu_pct)
            out.append(
                f" {Colors.BOLD}CPU{Colors.RESET}"
                f" {bc}[{'#' * int(cpu_pct / 100 * bar_len) + '.' * (bar_len - int(cpu_pct / 100 * bar_len))}]{Colors.RESET}"
                f" {bc}{cpu_pct:.0f}%{Colors.RESET}"
            )

        if mem_total_kb:
            bc = Colors.mem_bar(mem_pct)
            out.append(
                f" {Colors.BOLD}MEM{Colors.RESET}"
                f" {bc}[{'#' * int(mem_pct / 100 * bar_len) + '.' * (bar_len - int(mem_pct / 100 * bar_len))}]{Colors.RESET}"
                f" {bc}{mem_pct:.0f}%{Colors.RESET}"
                f" {Colors.GRAY}{fmt_size(mem_used_kb)}/{fmt_size(mem_total_kb)}{Colors.RESET}"
            )

        if disk_total:
            bc = Colors.disk_bar(disk_pct)
            out.append(
                f" {Colors.BOLD}DSK{Colors.RESET}"
                f" {bc}[{'#' * int(disk_pct / 100 * bar_len) + '.' * (bar_len - int(disk_pct / 100 * bar_len))}]{Colors.RESET}"
                f" {bc}{disk_pct:.0f}%{Colors.RESET}"
                f" {Colors.GRAY}{fmt_size(disk_used // 1024)}/{fmt_size(disk_total // 1024)}{Colors.RESET}"
            )

        net_rx, net_tx = net_curr
        net_rx_prev, net_tx_prev = net_prev
        rx_rate = (net_rx - net_rx_prev) / interval
        tx_rate = (net_tx - net_tx_prev) / interval
        show_net = rx_rate > 1024 or tx_rate > 1024
        if show_net:
            out.append(
                f" {Colors.BOLD}NET{Colors.RESET}"
                f" {Colors.GREEN}\u2191 {fmt_rate(tx_rate)}{Colors.RESET}"
                f"  {Colors.CYAN}\u2193 {fmt_rate(rx_rate)}{Colors.RESET}"
            )

        overhead = len(out) + 2
        max_procs = max(1, term.lines - overhead - 1)

        procs = get_top_procs(max_procs)

        out.append(f" {Colors.GRAY}{'─' * max(0, term.columns - 2)}{Colors.RESET}")
        out.append(
            f" {Colors.BOLD}{'PID':>5}  {'NAME':<12}  {'CPU':>4}  {'MEM':>7}{Colors.RESET}"
        )
        for pid, name, cpu, mem in procs:
            name_trunc = name if len(name) < 12 else name[:9] + "..."
            cpu_color = Colors.YELLOW if float(cpu or 0) >= 10 else Colors.WHITE
            out.append(
                f" {Colors.GRAY}{pid:>5}{Colors.RESET}"
                f"  {Colors.WHITE}{name_trunc:<12}{Colors.RESET}"
                f"  {cpu_color}{cpu:>5}{Colors.RESET}"
                f"  {Colors.CYAN}{mem:>5}MiB{Colors.RESET}"
            )

        sys.stdout.write("\033[H\033[J" + "\n".join(out) + "\n")
        sys.stdout.flush()

        cpu_prev = cpu_curr
        net_prev = net_curr
        time.sleep(interval)


if __name__ == "__main__":
    main()