import os
import sys
import tempfile

import pytest

sys.path.insert(0, os.path.dirname(os.path.dirname(os.path.abspath(__file__))))

from monitor import MonitorError, build_ssh_cmd, get_ssh_target, parse_toml_servers, require_commands, validate_user


@pytest.fixture
def tmp_toml(tmp_path):
    def _write(content):
        p = tmp_path / "config.toml"
        p.write_text(content)
        return str(p)
    return _write