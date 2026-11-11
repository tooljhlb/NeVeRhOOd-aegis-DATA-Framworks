"""Exercise optional retrieval/export through the real CLI and authenticated API."""

import argparse
import hashlib
import json
import os
import socket
import subprocess
import tempfile
import time
import urllib.error
import urllib.request
from pathlib import Path


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("binary", type=Path)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    binary = args.binary.resolve()
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)

        def cli(command, value, name):
            source, output = root / (name + ".input.json"), root / name
            source.write_text(json.dumps(value))
            subprocess.run([binary, command, source, output], check=True, capture_output=True)
            return output.read_bytes()

        documents = [
            {
                "source": "owned/manual",
                "revision": "r1",
                "license": "owned",
                "group_id": "manual",
                "text": "Optimizer checkpoints retain momentum.\n日本語",
            }
        ]
        index = json.loads(cli("knowledge-build", documents, "index.json"))
        request = {
            "prompt": "optimizer",
            "index": index,
            "expected_chunks_sha256": index["chunks_sha256"],
            "top_k": 4,
        }
        enabled = json.loads(cli("knowledge-context", request, "context.json"))
        assert enabled["citations"][0]["text"] == documents[0]["text"]
        disabled = json.loads(cli("knowledge-context", {"prompt": "optimizer"}, "off.json"))
        assert not disabled["retrieval_enabled"] and disabled["citations"] == []
        chats = [
            {
                "id": "session-1",
                "group_id": "session",
                "source": "owned/chat",
                "revision": "r1",
                "license": "owned",
                "approved_for_training": True,
                "messages": [
                    {"role": "user", "content": "What is saved?"},
                    {"role": "assistant", "content": "Optimizer state."},
                ],
            }
        ]
        exported = cli("chat-export", chats, "training.jsonl")
        assert json.loads(exported)["messages"] == chats[0]["messages"]
        with socket.socket() as listener:
            listener.bind(("127.0.0.1", 0))
            port = listener.getsockname()[1]
        token = "local-verification-token-0123456789"
        server = subprocess.Popen(
            [
                binary,
                "operator-serve",
                root / "ledger.db",
                root,
                "--bind",
                f"127.0.0.1:{port}",
                "--trusted-local",
            ],
            env={**os.environ, "AEGIS_OPERATOR_TOKEN": token},
            stdout=subprocess.DEVNULL,
            stderr=subprocess.PIPE,
        )

        def post(path, value, authenticated=True):
            headers = {"Content-Type": "application/json"}
            if authenticated:
                headers["Authorization"] = "Bearer " + token
            req = urllib.request.Request(
                f"http://127.0.0.1:{port}" + path, data=json.dumps(value).encode(), headers=headers
            )
            return json.load(urllib.request.urlopen(req, timeout=10))

        def rejected(path, value, code, authenticated=True):
            try:
                post(path, value, authenticated)
            except urllib.error.HTTPError as error:
                assert error.code == code, error.code
            else:
                raise AssertionError("request unexpectedly accepted")

        try:
            for _ in range(200):
                try:
                    with urllib.request.urlopen(f"http://127.0.0.1:{port}/health", timeout=1):
                        break
                except urllib.error.URLError:
                    if server.poll() is not None:
                        raise RuntimeError(server.stderr.read().decode())
                    time.sleep(0.05)
            else:
                raise AssertionError("server startup timed out")
            assert post("/v1/knowledge/build", documents) == index
            assert post("/v1/knowledge/context", request) == enabled
            assert post("/v1/knowledge/context", {"prompt": "optimizer"}) == disabled
            result = post("/v1/chats/export", chats)
            assert result["jsonl"].encode() == exported
            assert result["sha256"] == hashlib.sha256(exported).hexdigest()
            rejected("/v1/knowledge/context", request, 401, False)
            request["index"]["chunks"][0]["document"]["text"] += "tamper"
            rejected("/v1/knowledge/context", request, 400)
            chats[0]["approved_for_training"] = False
            rejected("/v1/chats/export", chats, 400)
            assert not result["training_started"]
        finally:
            server.terminate()
            server.wait(timeout=10)
    receipt = {
        "schema_version": 1,
        "retrieval_off_on_verified": True,
        "cli_api_parity": True,
        "tampered_index_rejected": True,
        "unapproved_export_rejected": True,
        "authentication_verified": True,
        "model_invoked": False,
        "training_started": False,
        "scope": "small owned-text fixture; lexical retrieval and generic chat JSONL, not model quality or training certification",
    }
    args.out.write_text(json.dumps(receipt, indent=2) + "\n")
    print(json.dumps(receipt))


if __name__ == "__main__":
    main()
