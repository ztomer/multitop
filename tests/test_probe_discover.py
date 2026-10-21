"""The discover probe's own rules, proved without the estate.

`tools/probe_discover.py` prints what a host advertises: the version its
tool table belongs to, and the tools it says it does not offer. Those two
are the catalogue's KEY, so the probe is what a reader checks a kept list
against - and a probe that asked for the wrong thing, or read the wrong
line of the answer, would check nothing.

Nothing here touches the network: the probe needs a host, and a test that
needs a host is a test that fails when the host is down.
"""

import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))

import probe_discover as probe  # noqa: E402


def test_the_probe_asks_for_the_discover_answer_in_this_protocol():
    """The request names `server/discover` and the version the client speaks."""
    sent = json.loads(probe.request())
    assert sent["jsonrpc"] == "2.0"
    assert sent["method"] == "server/discover"
    meta = sent["params"]["_meta"]
    assert meta["io.modelcontextprotocol/protocolVersion"] == probe.PROTO
    assert meta["io.modelcontextprotocol/clientCapabilities"] == {}
    assert "\n" not in probe.request(), "the request is one line"


def test_the_answer_is_the_last_thing_the_host_said():
    """A server may write a line of its own before it answers."""
    answer = {"jsonrpc": "2.0", "id": 1, "result": {"supportedVersions": ["2026-07-28"]}}
    out = "a line of the server's own\n" + json.dumps(answer) + "\n"
    assert probe.parse(out) == answer


def test_an_answer_that_is_not_json_is_refused_by_name():
    """A probe that cannot read the answer says so, rather than printing None."""
    try:
        probe.parse("not json at all")
    except ValueError as error:
        assert "not JSON" in str(error)
    else:
        raise AssertionError("an unreadable answer was accepted")


def test_an_empty_answer_is_refused_by_name():
    """A host that said nothing is not an answer."""
    try:
        probe.parse("")
    except ValueError as error:
        assert "not JSON" in str(error)
    else:
        raise AssertionError("an empty answer was accepted")
