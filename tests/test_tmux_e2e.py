import subprocess
import time
import unittest
import os
import tempfile

def print_info(message):
    print(f"[ ==> ] {message}")

def print_ok(message):
    print(f"[ Ok  ] {message}")

class TestMultitopE2E(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.tmux_sock = '/tmp/multitop_tmux_e2e.sock'
        cls.conf_path = '/tmp/multitop_e2e_config.toml'
        cls.run_cmd(f"tmux -S {cls.tmux_sock} kill-server 2>/dev/null || true")
        cls.run_cmd("rm -f ~/.cache/multitop/multitop.state")
        
        with open(cls.conf_path, 'w') as f:
            f.write("""
[[servers]]
host = "127.0.0.1"
port = 2222
user = "test"
upgrade_cmd = "printf 'Interactive prompt [Y/n] '; sleep 1.5; printf '10%%\\r20%%\\r30%%\\n'; sleep 0.5; echo 'mock done'"
""")
        
        cls.bin_path = cls.run_cmd("find ~/.cache/cargo-target -type f -name multitop -perm +111 | head -n 1").strip()
        cmd = f"{cls.bin_path} -c {cls.conf_path}"
        subprocess.Popen(["tmux", "-S", cls.tmux_sock, "new-session", "-d", "-x", "80", "-y", "24", cmd])
        time.sleep(2)
        
        print_info("Entering upgrade view (u)")
        cls.run_cmd(f"tmux -S {cls.tmux_sock} send-keys 'u'")
        time.sleep(0.5)
        print_info("Opening confirm modal (u)")
        cls.run_cmd(f"tmux -S {cls.tmux_sock} send-keys 'u'")
        time.sleep(0.5)
        print_info("Confirming upgrade (u)")
        cls.run_cmd(f"tmux -S {cls.tmux_sock} send-keys 'u'")
        time.sleep(1)

    @classmethod
    def tearDownClass(cls):
        cls.run_cmd(f"tmux -S {cls.tmux_sock} kill-server 2>/dev/null || true")
        if os.path.exists(cls.conf_path):
            os.remove(cls.conf_path)

    @staticmethod
    def run_cmd(cmd):
        return subprocess.check_output(cmd, shell=True).decode('utf-8')

    def capture_pane(self):
        return self.run_cmd(f"tmux -S {self.tmux_sock} capture-pane -p")

    def test_01_interactive_prompt_rendered(self):
        screen = self.capture_pane()
        self.assertIn("Interactive prompt [Y/n]", screen, "Fix B Failed: Incomplete line not rendered!")
        print_ok("Interactive prompt rendered correctly.")

    def test_02_switch_view_during_upgrade(self):
        print_info("Switching to stats view during upgrade (s)")
        self.run_cmd(f"tmux -S {self.tmux_sock} send-keys 's'")
        time.sleep(0.5)
        screen = self.capture_pane()
        self.assertIn("Stats", screen, "Fix A Failed: Could not switch to Stats view!")
        self.assertIn("CPU", screen, "Fix A Failed: Stats view not rendered properly!")
        print_ok("Switched away from Upgrade view successfully.")

    def test_03_carriage_returns_collapsed(self):
        print_info("Waiting for upgrade to finish and switching back...")
        time.sleep(1.5)
        self.run_cmd(f"tmux -S {self.tmux_sock} send-keys 'u'")
        time.sleep(1)
        screen = self.capture_pane()
        self.assertIn("30%", screen, "Fix B Failed: Carriage returns not collapsed!")
        self.assertNotIn("\n 10%", screen, "Fix B Failed: Old lines still present!")
        print_ok("Carriage returns collapsed correctly.")


    def test_04_docker_panel(self):
        print_info("Switching to Docker panel (d)")
        self.run_cmd(f"tmux -S {self.tmux_sock} send-keys 'd'")
        time.sleep(0.5)
        screen = self.capture_pane()
        self.assertTrue("CONTAINER ID" in screen or "No running containers" in screen, "Docker panel not rendered properly!")
        print_ok("Docker panel rendered correctly.")

    def test_05_fetch_panel(self):
        print_info("Switching to Fetch panel (f)")
        self.run_cmd(f"tmux -S {self.tmux_sock} send-keys 'f'")
        time.sleep(0.5)
        screen = self.capture_pane()
        self.assertTrue("OS" in screen and "Kernel" in screen, "Fetch panel not rendered properly!")
        print_ok("Fetch panel rendered correctly.")

    def test_06_graphs_panel(self):
        print_info("Switching to Graphs panel (g)")
        self.run_cmd(f"tmux -S {self.tmux_sock} send-keys 'g'")
        time.sleep(0.5)
        screen = self.capture_pane()
        self.assertTrue("CPU" in screen and "MEM" in screen and "NET" in screen, "Graphs panel not rendered properly!")
        print_ok("Graphs panel rendered correctly.")

    def test_07_settings_panel(self):
        print_info("Switching to Settings panel (e)")
        self.run_cmd(f"tmux -S {self.tmux_sock} send-keys 'e'")
        time.sleep(0.5)
        screen = self.capture_pane()
        self.assertIn("┌ Settings", screen, "Settings panel not rendered properly!")
        print_ok("Settings panel rendered correctly.")
        # escape back to normal view
        self.run_cmd(f"tmux -S {self.tmux_sock} send-keys 'Escape'")
        time.sleep(0.2)

    def test_08_filter_panel(self):
        print_info("Activating Filter (/)")
        self.run_cmd(f"tmux -S {self.tmux_sock} send-keys '/'")
        time.sleep(0.5)
        screen = self.capture_pane()
        self.assertIn("Filter", screen, "Filter prompt not active!")
        # Type something
        self.run_cmd(f"tmux -S {self.tmux_sock} send-keys 'mock'")
        time.sleep(0.5)
        screen = self.capture_pane()
        self.assertIn("mock", screen, "Filter input not working!")
        print_ok("Filter panel rendered correctly.")
        self.run_cmd(f"tmux -S {self.tmux_sock} send-keys 'Escape'")
        time.sleep(0.2)

if __name__ == '__main__':
    unittest.main(verbosity=2)
