"""Deterministic protocol fixture. This is NOT a model or an inference backend."""

import json
import os
import sys
import time
from pathlib import Path


def receive():
    length = None
    while True:
        line = sys.stdin.buffer.readline()
        if not line:
            raise EOFError
        if line == b"\r\n":
            break
        if line.lower().startswith(b"content-length:"):
            length = int(line.split(b":", 1)[1])
    return json.loads(sys.stdin.buffer.read(length))


def send(value):
    payload = json.dumps(value).encode()
    sys.stdout.buffer.write(f"Content-Length: {len(payload)}\r\n\r\n".encode() + payload)
    sys.stdout.buffer.flush()


def main():
    mode = sys.argv[1] if len(sys.argv) > 1 else "normal"
    if len(sys.argv) > 2:
        Path(sys.argv[2]).write_text(str(os.getpid()))
    if mode == "hang":
        time.sleep(60)
        return
    hello = receive()
    identity = hello["params"]
    send(
        {
            "jsonrpc": "2.0",
            "id": 1,
            "result": {
                "protocol_version": 1,
                "model_id": identity["model_id"],
                "model_revision": identity["model_revision"],
                "capabilities": {"streaming": True, "tool_calls": True},
            },
        }
    )
    receive()
    send(
        {
            "jsonrpc": "2.0",
            "method": "aegis.delta",
            "params": {"request_id": 2, "text": "contract "},
        }
    )
    if mode == "slow":
        time.sleep(60)
    send(
        {"jsonrpc": "2.0", "method": "aegis.delta", "params": {"request_id": 2, "text": "fixture"}}
    )
    calls = (
        [{"id": "one", "name": "undeclared_tool", "arguments": {}}] if mode == "bad-tool" else []
    )
    send(
        {
            "jsonrpc": "2.0",
            "id": 2,
            "result": {
                "text": "contract fixture" if mode != "bad-final" else "wrong",
                "tool_calls": calls,
                "finish_reason": "tool_calls" if calls else "stop",
            },
        }
    )


if __name__ == "__main__":
    main()
