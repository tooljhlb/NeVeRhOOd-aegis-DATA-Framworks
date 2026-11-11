"""Exercise API capacity admission against actual isolated container runs."""

import argparse
import json
import os
import socket
import subprocess
import tempfile
import time
import urllib.error
import urllib.request
from pathlib import Path

parser = argparse.ArgumentParser()
parser.add_argument("binary", type=Path)
parser.add_argument("--image", required=True)
parser.add_argument("--out", type=Path, required=True)
a = parser.parse_args()
with tempfile.TemporaryDirectory() as tmp:
    root = Path(tmp)
    profile = {
        "schema_version": 1,
        "id": "capacity-test",
        "require_container": True,
        "allowed_programs": ["sh"],
        "allowed_images": [a.image],
        "max_slots": 1,
        "max_wall_seconds": 30,
        "max_attempts": 1,
        "max_stage_seconds": 20,
        "max_cpu": 1,
        "max_memory_mib": 128,
    }
    (root / "profile.json").write_text(json.dumps(profile))
    (root / "capacity.json").write_text(json.dumps({"max_runs": 2, "cpu": 1, "memory_mib": 128}))
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        port = sock.getsockname()[1]
    env = dict(
        os.environ,
        AEGIS_RUNTIME_PROFILE=str(root / "profile.json"),
        AEGIS_API_CAPACITY=str(root / "capacity.json"),
        AEGIS_OPERATOR_TOKEN="capacity-test-token-abcdefghijklmnopqrstuvwxyz",
    )
    process = subprocess.Popen(
        [
            str(a.binary.resolve()),
            "operator-serve",
            str(root / "api.db"),
            tmp,
            "--bind",
            f"127.0.0.1:{port}",
            "--trusted-local",
        ],
        env=env,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
    )

    def request(path, data=None):
        req = urllib.request.Request(
            f"http://127.0.0.1:{port}{path}",
            data=None if data is None else json.dumps(data).encode(),
            headers={
                "Authorization": f"Bearer {env['AEGIS_OPERATOR_TOKEN']}",
                "Content-Type": "application/json",
            },
        )
        with urllib.request.urlopen(req, timeout=3) as response:
            return json.load(response)

    def wait_for(predicate):
        for _ in range(400):
            try:
                if predicate():
                    return
            except (OSError, KeyError):
                pass
            time.sleep(0.05)
        raise AssertionError("timed out waiting for API condition")

    def plan(run):
        workspace = root / run
        workspace.mkdir()
        return {
            "run_id": run,
            "workspace": str(workspace),
            "mode": "bounded",
            "max_slots": 1,
            "max_wall_seconds": 30,
            "allowed_operations": ["test"],
            "stages": [
                {
                    "id": "sleep",
                    "operation": "test",
                    "program": "sh",
                    "args": ["-c", "sleep 3; printf verified > output.txt"],
                    "outputs": ["output.txt"],
                    "timeout_seconds": 20,
                    "container": {"image": a.image, "cpus": 1, "memory_mib": 128, "pids": 32},
                }
            ],
        }

    try:
        wait_for(lambda: request("/v1/runtime")["capacity"]["reserved"]["runs"] == 0)
        first, second = plan("first"), plan("second")
        request("/v1/runs", first)
        assert request("/v1/runtime")["capacity"]["reserved"]["cpu"] == 1
        try:
            request("/v1/runs", second)
            raise AssertionError("capacity overbooking was accepted")
        except urllib.error.HTTPError as error:
            assert error.code == 429
        wait_for(lambda: request("/v1/runs/first")["status"] == "completed")
        wait_for(lambda: request("/v1/runtime")["capacity"]["reserved"]["runs"] == 0)
        request("/v1/runs", second)
        wait_for(lambda: request("/v1/runs/second")["status"] == "completed")
        wait_for(lambda: request("/v1/runtime")["capacity"]["reserved"]["runs"] == 0)
        assert (root / "first/output.txt").read_text() == "verified"
        assert (root / "second/output.txt").read_text() == "verified"
    finally:
        process.terminate()
        process.wait(timeout=10)
receipt = {
    "isolated_execution": True,
    "overbooking_rejected_429": True,
    "capacity_released_after_completion": True,
    "subsequent_execution": True,
    "scope": "one API process with actual OCI stages",
}
a.out.write_text(json.dumps(receipt, indent=2) + "\n")
print(json.dumps(receipt))
