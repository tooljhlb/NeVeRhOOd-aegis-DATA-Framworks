"""Discover and execute a repository component through consistent CLI/API contracts."""

import argparse
import json
import os
import shutil
import socket
import subprocess
import tempfile
import time
import urllib.error
import urllib.request
from pathlib import Path

p = argparse.ArgumentParser()
p.add_argument("binary", type=Path)
p.add_argument("--out", type=Path, required=True)
a = p.parse_args()
binary = a.binary.resolve()
repo = Path(__file__).resolve().parents[1]
with tempfile.TemporaryDirectory() as tmp:
    root = Path(tmp)
    project = root / "project"
    (project / "native/tests").mkdir(parents=True)
    for name in ["CMakeLists.txt", "native/reference.hpp", "native/tests/reference_test.cpp"]:
        shutil.copyfile(repo / name, project / name)

    def call(*args):
        r = subprocess.run(
            [str(binary), *map(str, args)], capture_output=True, text=True, timeout=120
        )
        assert r.returncode == 0, r.stdout + r.stderr
        return json.loads(r.stdout)

    generated = root / "generated"
    discovered = call("project-discover", project, generated, "--run-id", "cli-discovered")
    assert discovered["runnable_plan_generated"] and discovered["execution"] == "not_started"
    assert len(discovered["plan"]["stages"]) == 3
    call(
        "project-run",
        generated / "profile.json",
        generated / "plan.json",
        root / "cli.db",
        root / "export",
        "--trusted-local",
    )
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        port = s.getsockname()[1]
    token = "discovery-token-abcdefghijklmnopqrstuvwxyz"
    env = dict(os.environ, AEGIS_OPERATOR_TOKEN=token)
    process = subprocess.Popen(
        [
            str(binary),
            "operator-serve",
            str(root / "api.db"),
            str(project),
            "--bind",
            f"127.0.0.1:{port}",
            "--trusted-local",
        ],
        env=env,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
    )

    def request(path, data=None, bearer=token):
        req = urllib.request.Request(
            f"http://127.0.0.1:{port}{path}",
            data=None if data is None else json.dumps(data).encode(),
            headers={"Authorization": f"Bearer {bearer}", "Content-Type": "application/json"},
        )
        with urllib.request.urlopen(req, timeout=10) as response:
            return json.load(response)

    try:
        for _ in range(100):
            try:
                request("/health")
                break
            except OSError:
                time.sleep(0.05)
        report = request(
            "/v1/projects/discover", {"workspace": str(project), "run_id": "api-discovered"}
        )
        assert report["plan"]["stages"] == discovered["plan"]["stages"]
        request("/v1/runs", report["plan"])
        for _ in range(400):
            try:
                if request("/v1/runs/api-discovered")["status"] == "completed":
                    break
            except urllib.error.HTTPError:
                pass
            time.sleep(0.05)
        else:
            raise AssertionError("API-generated run did not complete")
        assert call("operator-status", root / "api.db", "api-discovered")["status"] == "completed"
        for data, bearer, code in [
            ({"workspace": str(root), "run_id": "outside"}, token, 400),
            ({"workspace": str(project), "run_id": "unauthorized"}, "wrong", 401),
        ]:
            try:
                request("/v1/projects/discover", data, bearer)
                raise AssertionError("discovery boundary bypass")
            except urllib.error.HTTPError as error:
                assert error.code == code
        with urllib.request.urlopen(f"http://127.0.0.1:{port}/", timeout=5) as response:
            assert "discover-project" in response.read().decode()
    finally:
        process.terminate()
        process.wait(timeout=10)
receipt = {
    "generated_without_execution": True,
    "real_cpp_component_discovered_built_tested_and_exported": True,
    "cli_api_discovery_contract_matches": True,
    "api_task_inspected_through_cli": True,
    "authentication_and_root_boundary_enforced": True,
    "cockpit_discovery_control_present": True,
    "scope": "trusted local repository component; not full editor, LSP/DAP or universal project compatibility",
}
a.out.write_text(json.dumps(receipt, indent=2) + "\n")
print(json.dumps(receipt))
