"""Exercise shared runtime admission through the actual CLI and HTTP API."""

import json
import os
import socket
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
from pathlib import Path

binary = str(Path(sys.argv[1]).resolve())
with tempfile.TemporaryDirectory() as tmp:
    root = Path(tmp)
    profile = {
        "schema_version": 1,
        "id": "host-test",
        "require_container": False,
        "allowed_programs": ["sh"],
        "allowed_images": [],
        "max_slots": 1,
        "max_wall_seconds": 30,
        "max_attempts": 1,
        "max_stage_seconds": 10,
        "max_cpu": 1,
        "max_memory_mib": 128,
    }
    plan = {
        "run_id": "runtime-cli",
        "workspace": tmp,
        "mode": "bounded",
        "max_slots": 1,
        "max_wall_seconds": 30,
        "allowed_operations": ["test"],
        "stages": [
            {
                "id": "test",
                "operation": "test",
                "program": "sh",
                "args": ["-c", "printf verified > output.txt"],
                "outputs": ["output.txt"],
                "timeout_seconds": 10,
            }
        ],
    }
    p = root / "profile.json"
    f = root / "plan.json"
    p.write_text(json.dumps(profile))
    f.write_text(json.dumps(plan))
    subprocess.run(
        [binary, "runtime-run", str(p), str(f), str(root / "cli.db"), "--trusted-local"],
        check=True,
        capture_output=True,
    )
    assert (root / "output.txt").read_text() == "verified"
    plan["max_slots"] = 2
    f.write_text(json.dumps(plan))
    assert (
        subprocess.run([binary, "runtime-check", str(p), str(f)], capture_output=True).returncode
        != 0
    )
    plan["max_slots"] = 1
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        port = sock.getsockname()[1]
    token = "runtime-test-token-abcdefghijklmnopqrstuvwxyz"
    env = dict(os.environ, AEGIS_RUNTIME_PROFILE=str(p), AEGIS_OPERATOR_TOKEN=token)
    process = subprocess.Popen(
        [
            binary,
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
            headers={"Authorization": f"Bearer {token}", "Content-Type": "application/json"},
        )
        with urllib.request.urlopen(req, timeout=3) as response:
            return json.load(response)

    try:
        for _ in range(100):
            try:
                state = request("/v1/runtime")
                break
            except OSError:
                time.sleep(0.05)
        else:
            raise AssertionError("API did not start")
        assert state["profile"]["id"] == profile["id"]
        plan["run_id"] = "runtime-api"
        plan["stages"][0]["program"] = "bash"
        try:
            request("/v1/runs", plan)
            raise AssertionError("API accepted forbidden executable")
        except urllib.error.HTTPError as error:
            assert error.code == 400
        plan["stages"][0]["program"] = "sh"
        receipt = request("/v1/runs", plan)
        assert len(receipt["admission"]["profile_sha256"]) == 64
        for _ in range(100):
            try:
                status = request("/v1/runs/runtime-api")
                if status["status"] == "completed":
                    break
            except urllib.error.HTTPError:
                pass
            time.sleep(0.05)
        else:
            raise AssertionError("admitted API run did not complete")
    finally:
        process.terminate()
        process.wait(timeout=10)
print(
    json.dumps(
        {
            "cli_execution": True,
            "cli_budget_rejection": True,
            "api_policy_rejection": True,
            "api_admitted_execution": True,
        }
    )
)
