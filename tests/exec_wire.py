"""The exec channel's wire format, in Python, for the live end-to-end tests.

A second implementation on purpose. The Rust tests round-trip the codec against
itself, which proves it is self-consistent and nothing more -- a field written
and read in the same wrong order passes every one of them. This one is written
from the layout as documented, so a disagreement between the two is a
disagreement between the code and its own specification.
"""

import struct

MAGIC = b"MTOP"
VERSION = 5
MODE_EXEC = 3
MODE_HELLO = 4

KIND_REQUEST, KIND_BEGIN, KIND_OUT, KIND_MARKER, KIND_ALIVE, KIND_EXIT = range(6)
MARKERS = {0: "PwReady", 1: "SudoFailed", 2: "LockHeld", 3: "Started", 4: "Done"}
STREAMS = {0: "stdout", 1: "stderr"}


def _str(raw):
    return struct.pack("<H", len(raw)) + raw


def request(command, password=None, use_lock=False, cols=80, rows=24):
    """Build the frame that tells an agent what to run."""
    body = bytes([KIND_REQUEST]) + _str(command.encode())
    if password is None:
        body += bytes([0])
    else:
        body += bytes([1]) + _str(password.encode())
    body += bytes([1 if use_lock else 0]) + struct.pack("<HH", cols, rows)
    return MAGIC + bytes([VERSION, MODE_EXEC]) + struct.pack("<H", len(body)) + body


class Desync(Exception):
    """The stream stopped being frames. Never silently tolerated: reading past a
    bad frame is how a reader invents plausible output that was never sent."""


def decode(data):
    """Split a reply stream into frames, strictly."""
    frames, pos = [], 0
    while pos + 8 <= len(data):
        if data[pos:pos + 4] != MAGIC:
            raise Desync(f"no magic at byte {pos}: {data[pos:pos + 16]!r}")
        ver, mode = data[pos+4], data[pos+5]
        length = struct.unpack("<H", data[pos + 6:pos + 8])[0]
        body = data[pos + 8:pos + 8 + length]
        if len(body) < length:
            raise Desync(f"frame at {pos} claims {length} bytes, {len(body)} present")
        if mode == MODE_HELLO:
            frames.append(_hello(body))
        elif mode == MODE_EXEC:
            frames.append(_frame(body))
        else:
            raise Desync(f"unexpected mode {mode} at byte {pos}")
        pos += 8 + length
    if pos != len(data):
        raise Desync(f"{len(data) - pos} trailing bytes that are not a frame")
    return frames


def _hello(body):
    ver, rest = _take_str(body)
    proto, min_proto = rest[0], rest[1]
    return ("Hello", ver.decode(), proto, min_proto)


def _frame(body):
    kind, rest = body[0], body[1:]
    if kind == KIND_BEGIN:
        host, rest = _take_str(rest)
        version, rest = _take_str(rest)
        return ("Begin", host.decode(), version.decode())
    if kind == KIND_OUT:
        stream = STREAMS[rest[0]]
        seq = struct.unpack("<I", rest[1:5])[0]
        blob, _ = _take_str(rest[5:])
        return ("Out", stream, seq, blob)
    if kind == KIND_MARKER:
        return ("Marker", MARKERS[rest[0]])
    if kind == KIND_ALIVE:
        return ("Alive", struct.unpack("<I", rest[:4])[0])
    if kind == KIND_EXIT:
        return ("Exit", struct.unpack("<i", rest[:4])[0], bool(rest[4]))
    raise Desync(f"unknown frame kind {kind}")


def _take_str(rest):
    length = struct.unpack("<H", rest[:2])[0]
    return rest[2:2 + length], rest[2 + length:]


def output(frames, stream="stdout"):
    """Everything one stream carried, in sequence order."""
    parts = sorted(
        ((f[2], f[3]) for f in frames if f[0] == "Out" and f[1] == stream),
        key=lambda p: p[0],
    )
    return b"".join(blob for _, blob in parts)


def exit_code(frames):
    """The run's outcome. Absent is a distinct answer from zero: a run that
    never reported finishing is the defect this channel was built to end."""
    for frame in reversed(frames):
        if frame[0] == "Exit":
            return frame[1]
    return None
