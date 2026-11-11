"""Real Ruff LSP and fixture-only model transport: CLI/API, boundaries, cancellation."""

import argparse
import hashlib
import json
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
from pathlib import Path


def sha(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("binary", type=Path)
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    binary = args.binary.resolve()
    fixture = (
        Path(__file__).resolve().parents[1] / "fixtures/adapters/contract_model.py"
    ).resolve()
    ruff = Path(shutil.which("ruff") or "missing-ruff").resolve()
    if not ruff.is_file():
        raise RuntimeError("Ruff is required; this verification cannot silently skip real LSP")
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        (root / "sample.py").write_text("import os\nprint(undefined_name)\n")
        original = sha(root / "sample.py")
        language = {
            "schema_version": 1,
            "id": "ruff",
            "program": str(ruff),
            "program_sha256": sha(ruff),
            "args": ["server"],
            "language_id": "python",
            "timeout_seconds": 15,
        }
        query = {"path": "sample.py", "operation": "diagnostics"}

        def write(name, value):
            path = root / name
            path.write_text(json.dumps(value))
            return path

        language_path = write("language.json", language)
        query_path = write("query.json", query)
        result_path = root / "language-result.json"
        subprocess.run(
            [
                binary,
                "language-query",
                root,
                language_path,
                query_path,
                result_path,
                "--trusted-local",
            ],
            check=True,
            capture_output=True,
            timeout=25,
        )
        result = json.loads(result_path.read_text())
        codes = {d["code"] for d in result["result"]["items"]}
        assert "F821" in codes and "F401" in codes
        assert result["source_sha256"] == original and not result["edits_applied"]
        adapter = {
            "schema_version": 1,
            "id": "fixture",
            "model_id": "contract-fixture-not-a-model",
            "model_revision": "v1",
            "program": str(Path(sys.executable).resolve()),
            "program_sha256": sha(sys.executable),
            "args": ["-u", str(fixture), "normal"],
            "input_files": {str(fixture): sha(fixture)},
            "timeout_seconds": 10,
        }
        request = {
            "messages": [{"role": "user", "content": "Check transport"}],
            "max_output_tokens": 32,
        }
        adapter_path = write("adapter.json", adapter)
        request_path = write("request.json", request)
        subprocess.run(
            [
                binary,
                "model-run",
                root,
                adapter_path,
                request_path,
                root / "model-result",
                "--trusted-local",
            ],
            check=True,
            capture_output=True,
            timeout=15,
        )
        cli_receipt = json.loads((root / "model-result/receipt.json").read_text())
        assert cli_receipt["result"]["text"] == "contract fixture"
        assert not cli_receipt["tool_calls_executed"]
        for mode in ["bad-tool", "bad-final", "hang"]:
            bad = {**adapter, "args": ["-u", str(fixture), mode], "timeout_seconds": 1}
            before = time.monotonic()
            completed = subprocess.run(
                [
                    binary,
                    "model-run",
                    root,
                    write(mode + ".json", bad),
                    request_path,
                    root / mode,
                    "--trusted-local",
                ],
                capture_output=True,
                timeout=6,
            )
            assert completed.returncode != 0 and not (root / mode / "receipt.json").exists()
            assert time.monotonic() - before < 5
        slow_pid = root / "slow.pid"
        slow = {
            **adapter,
            "id": "slow",
            "args": ["-u", str(fixture), "slow", str(slow_pid)],
            "timeout_seconds": 30,
        }
        language_configs = write("language-servers.json", [language])
        model_configs = write("model-adapters.json", [adapter, slow])
        with socket.socket() as listener:
            listener.bind(("127.0.0.1", 0))
            port = listener.getsockname()[1]
        token = "local-agent-interface-token-0123456789"
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
            env={
                **os.environ,
                "AEGIS_OPERATOR_TOKEN": token,
                "AEGIS_LANGUAGE_SERVERS": str(language_configs),
                "AEGIS_MODEL_ADAPTERS": str(model_configs),
            },
            stdout=subprocess.DEVNULL,
            stderr=subprocess.PIPE,
        )

        def post(path, value, authenticated=True):
            headers = {"Content-Type": "application/json"}
            if authenticated:
                headers["Authorization"] = "Bearer " + token
            return urllib.request.urlopen(
                urllib.request.Request(
                    f"http://127.0.0.1:{port}" + path,
                    data=json.dumps(value).encode(),
                    headers=headers,
                ),
                timeout=20,
            )

        def rejected(path, value, status, authenticated=True):
            try:
                with post(path, value, authenticated):
                    raise AssertionError("unexpected acceptance")
            except urllib.error.HTTPError as error:
                assert error.code == status, error.code

        def event(response):
            while True:
                line = response.readline()
                if not line:
                    return None
                if line.startswith(b"data: "):
                    return json.loads(line[6:])

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
                raise AssertionError("server did not start")
            with post("/v1/language/query", {"server_id": "ruff", "query": query}) as response:
                api_result = json.load(response)
            assert api_result["result"] == result["result"]
            rejected("/v1/language/query", {"server_id": "ruff", "query": query}, 401, False)
            rejected("/v1/language/query", {"server_id": "missing", "query": query}, 400)
            rejected(
                "/v1/language/query",
                {
                    "server_id": "ruff",
                    "query": {"path": "../escape.py", "operation": "diagnostics"},
                },
                400,
            )
            with post(
                "/v1/models/stream", {"adapter_id": "fixture", "request": request}
            ) as response:
                events = []
                while (item := event(response)) is not None:
                    events.append(item)
            assert events[0]["type"] == "started"
            assert events[-1]["type"] == "completed", events
            assert events[-1]["receipt"] == cli_receipt
            assert "".join(e["text"] for e in events if e["type"] == "delta") == "contract fixture"
            rejected("/v1/models/stream", {"adapter_id": "fixture", "request": request}, 401, False)
            response = post("/v1/models/stream", {"adapter_id": "slow", "request": request})
            assert event(response)["type"] == "started"
            assert event(response)["type"] == "delta"
            pid = int(slow_pid.read_text())
            response.close()
            for _ in range(100):
                try:
                    os.kill(pid, 0)
                except ProcessLookupError:
                    break
                time.sleep(0.05)
            else:
                raise AssertionError("disconnected model adapter was not killed/reaped")
            assert sha(root / "sample.py") == original
        finally:
            server.terminate()
            server.wait(timeout=10)
    receipt = {
        "schema_version": 1,
        "real_ruff_diagnostics": True,
        "lsp_cli_api_parity": True,
        "model_fixture_stream_cli_api_parity": True,
        "undeclared_tool_and_inconsistent_final_rejected": True,
        "hung_adapter_deadline_verified": True,
        "disconnect_kills_adapter": True,
        "source_unchanged": True,
        "scope": "real Ruff diagnostics; deterministic model protocol fixture only, no Aegis weights or model capability measurement",
    }
    args.out.write_text(json.dumps(receipt, indent=2) + "\n")
    print(json.dumps(receipt))


if __name__ == "__main__":
    main()
