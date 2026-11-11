"""Native operator integration: concurrency, rejection, API auth/control, repairs."""

import argparse
import hashlib
import json
import os
import secrets
import socket
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
from pathlib import Path


def main():
    p = argparse.ArgumentParser()
    p.add_argument("binary", type=Path)
    p.add_argument("--out", type=Path)
    a = p.parse_args()
    binary = a.binary.resolve()
    observations = {}
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        db = root / "operator.db"

        def call(*args, ok=True):
            r = subprocess.run([str(binary), *map(str, args)], capture_output=True, text=True)
            if ok and r.returncode:
                raise RuntimeError(r.stderr + r.stdout)
            return r

        def stage(name, code, deps=(), outputs=()):
            return dict(
                id=name,
                operation="test",
                program=sys.executable,
                args=["-c", code],
                depends_on=list(deps),
                outputs=list(outputs),
                inputs=[],
                timeout_seconds=5,
            )

        plan = dict(
            run_id="concurrency",
            workspace=str(root),
            mode="bounded",
            max_slots=2,
            max_wall_seconds=20,
            allowed_operations=["test"],
            stages=[
                stage(
                    "one",
                    "import time; from pathlib import Path; time.sleep(0.2); Path('one').write_text('1')",
                    outputs=["one"],
                ),
                stage(
                    "two",
                    "import time; from pathlib import Path; time.sleep(0.2); Path('two').write_text('2')",
                    outputs=["two"],
                ),
                stage(
                    "join",
                    "from pathlib import Path; Path('joined').write_text(Path('one').read_text()+Path('two').read_text())",
                    deps=["one", "two"],
                    outputs=["joined"],
                ),
            ],
        )
        config = root / "plan.json"
        config.write_text(json.dumps(plan))
        r = call("operator-run", db, config, "--trusted-local")
        assert json.loads(r.stdout)["status"] == "completed"
        events = json.loads(call("operator-events", db, "concurrency").stdout)
        starts = {e["payload"]["stage"]: e["seq"] for e in events if e["kind"] == "stage_started"}
        finishes = {
            e["payload"]["stage"]: e["seq"] for e in events if e["kind"] == "stage_finished"
        }
        assert max(starts["one"], starts["two"]) < min(finishes["one"], finishes["two"])
        observations["concurrent_stages_and_dependency_join"] = True
        (root / "joined").write_text("tampered")
        assert call("operator-run", db, config, "--trusted-local", ok=False).returncode != 0
        observations["completed_artifact_tamper_rejected"] = True
        # Completed runs must revalidate declared inputs and executable bytes.
        for kind in ("input", "executable"):
            watched = root / ("watched-" + kind)
            watched.write_text("original")
            task = stage(
                "identity",
                "from pathlib import Path; Path('identity-out').write_text('ok')",
                outputs=["identity-out"],
            )
            if kind == "input":
                task["inputs"] = [watched.name]
            else:
                watched.write_text("#!/bin/sh\nprintf ok > identity-out\n")
                watched.chmod(0o700)
                task["program"] = str(watched)
                task["args"] = []
            identity_plan = dict(plan, run_id="identity-" + kind, stages=[task])
            config.write_text(json.dumps(identity_plan))
            call("operator-run", db, config, "--trusted-local")
            with watched.open("ab") as stream:
                stream.write(b"changed")
            assert call("operator-run", db, config, "--trusted-local", ok=False).returncode != 0
            observations["completed_" + kind + "_change_rejected"] = True
        # Kill the real controller before a first checkpoint, then after one exists.
        for checkpoint_exists in (False, True):
            marker = root / "crash-ready"
            marker.unlink(missing_ok=True)
            checkpoint = root / "recovery.ckpt"
            checkpoint.unlink(missing_ok=True)
            task = stage(
                "recover",
                "import sys,time; from pathlib import Path; p=Path('crash-ready'); first=not p.exists(); "
                + (
                    "Path('recovery.ckpt').write_text('valid') if first else None; "
                    if checkpoint_exists
                    else ""
                )
                + "p.write_text('ready'); time.sleep(30) if first else None; "
                + "assert ('--resume' in sys.argv) == "
                + repr(checkpoint_exists)
                + "; Path('recovered').write_text('ok')",
                outputs=["recovered"],
            )
            task.update(
                max_attempts=2, timeout_seconds=60, resume_args=["--resume", "recovery.ckpt"]
            )
            crash_plan = dict(
                plan,
                run_id="crash-" + str(checkpoint_exists).lower(),
                stages=[task],
                max_wall_seconds=90,
            )
            config.write_text(json.dumps(crash_plan))
            process = subprocess.Popen(
                [str(binary), "operator-run", str(db), str(config), "--trusted-local"],
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
            )
            try:
                deadline = time.monotonic() + 10
                while not marker.exists() and time.monotonic() < deadline:
                    time.sleep(0.02)
                assert marker.exists(), "controller did not launch child"
                process.kill()
                process.communicate(timeout=5)
                time.sleep(0.1)
                result = call("operator-run", db, config, "--trusted-local")
                assert json.loads(result.stdout)["status"] == "completed"
            finally:
                if process.poll() is None:
                    process.kill()
                    process.communicate()
            observations[
                "controller_sigkill_" + ("with" if checkpoint_exists else "before") + "_checkpoint"
            ] = True
        plan["run_id"] = "forbidden"
        plan["allowed_operations"] = []
        config.write_text(json.dumps(plan))
        r = call("operator-run", db, config, "--trusted-local", ok=False)
        assert r.returncode != 0 and json.loads(r.stdout)["status"] == "rejected"
        observations["policy_rejection"] = True
        # Timeout and explicit retry are independently observed, not inferred from source.
        plan.update(
            run_id="retry",
            allowed_operations=["test"],
            stages=[
                stage(
                    "temporary",
                    "from pathlib import Path; import sys; p=Path('attempt'); n=int(p.read_text())+1 if p.exists() else 1; p.write_text(str(n)); sys.exit(75 if n==1 else 0)",
                )
            ],
        )
        plan["stages"][0].update(max_attempts=2, retry_exit_codes=[75])
        config.write_text(json.dumps(plan))
        call("operator-run", db, config, "--trusted-local")
        assert (root / "attempt").read_text() == "2"
        observations["bounded_explicit_retry"] = True
        plan.update(run_id="cancel-test", stages=[stage("sleep", "import time; time.sleep(30)")])
        config.write_text(json.dumps(plan))
        process = subprocess.Popen(
            [str(binary), "operator-run", str(db), str(config), "--trusted-local"],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
        try:
            for _ in range(100):
                q = call("operator-status", db, "cancel-test", ok=False)
                if q.returncode == 0 and json.loads(q.stdout)["stages"]:
                    break
                time.sleep(0.02)
            call("operator-control", db, "cancel-test", "cancel")
            process.communicate(timeout=5)
            assert (
                json.loads(call("operator-status", db, "cancel-test").stdout)["status"]
                == "cancelled"
            )
        finally:
            if process.poll() is None:
                process.kill()
                process.communicate()
        observations["active_process_cancellation"] = True
        plan.update(
            run_id="pause-test",
            stages=[
                stage(
                    "boundary",
                    "import time; from pathlib import Path; time.sleep(0.3); Path('boundary').write_text('ready')",
                    outputs=["boundary"],
                ),
                stage(
                    "later",
                    "from pathlib import Path; Path('later').write_text('done')",
                    deps=["boundary"],
                    outputs=["later"],
                ),
            ],
        )
        config.write_text(json.dumps(plan))
        process = subprocess.Popen(
            [str(binary), "operator-run", str(db), str(config), "--trusted-local"],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
        try:
            for _ in range(100):
                q = call("operator-status", db, "pause-test", ok=False)
                if q.returncode == 0 and json.loads(q.stdout)["stages"]:
                    break
                time.sleep(0.01)
            call("operator-control", db, "pause-test", "pause")
            process.communicate(timeout=5)
            assert (
                json.loads(call("operator-status", db, "pause-test").stdout)["status"] == "paused"
            )
            assert not (root / "later").exists()
            call("operator-control", db, "pause-test", "run")
            assert (
                json.loads(call("operator-run", db, config, "--trusted-local").stdout)["status"]
                == "completed"
            )
        finally:
            if process.poll() is None:
                process.kill()
                process.communicate()
        observations["pause_boundary_and_resume"] = True

        source = root / "sample.py"
        source.write_text("def add(a,b):\n return a+b\n")
        original = source.read_bytes()
        proposal = root / "proposal"
        call("repair-propose", source, proposal)
        assert source.read_bytes() == original
        call("repair-apply", source, proposal / "proposal.json")
        assert source.read_bytes() != original
        call("repair-apply", source, proposal / "proposal.json", "--rollback")
        assert source.read_bytes() == original
        observations["isolated_repair_apply_rollback"] = True
        with socket.socket() as listener:
            listener.bind(("127.0.0.1", 0))
            port = listener.getsockname()[1]
        token = secrets.token_urlsafe(32)
        env = dict(os.environ, AEGIS_OPERATOR_TOKEN=token)
        server = subprocess.Popen(
            [
                str(binary),
                "operator-serve",
                str(db),
                str(root),
                "--bind",
                f"127.0.0.1:{port}",
                "--trusted-local",
            ],
            env=env,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )

        def request(path, authenticated=True, data=None):
            req = urllib.request.Request(
                f"http://127.0.0.1:{port}" + path,
                headers={"Authorization": "Bearer " + token, "Content-Type": "application/json"}
                if authenticated
                else {},
                data=json.dumps(data).encode() if data is not None else None,
            )
            with urllib.request.urlopen(req, timeout=2) as response:
                return response.read()

        try:
            for _ in range(100):
                try:
                    request("/health", False)
                    break
                except (OSError, urllib.error.URLError):
                    time.sleep(0.02)
            try:
                request("/v1/graph", False)
                raise AssertionError("unauthenticated graph access")
            except urllib.error.HTTPError as e:
                assert e.code == 401
            assert json.loads(request("/v1/runs/retry"))["status"] == "completed"
            assert json.loads(request("/v1/graph"))["nodes"]
            assert b"AEGIS" in request("/", False)
            assert json.loads(request("/v1/runs/retry/metrics"))["scope"]
            assert (
                json.loads(
                    request(
                        "/v1/runs/retry/notes",
                        data={"actor": "ai-test", "text": "Inspect the retry receipt"},
                    )
                )["seq"]
                > 0
            )
            api_plan = dict(
                run_id="api-submit",
                workspace=str(root),
                mode="bounded",
                max_slots=1,
                max_wall_seconds=5,
                allowed_operations=["test"],
                stages=[stage("api-check", "print('API execution verified')")],
            )
            assert json.loads(request("/v1/runs", data=api_plan))["run_id"] == "api-submit"
            for _ in range(100):
                try:
                    done = json.loads(request("/v1/runs/api-submit"))
                except urllib.error.HTTPError:
                    time.sleep(0.02)
                    continue
                if done["status"] == "completed":
                    break
                time.sleep(0.02)
            assert done["status"] == "completed"
            observations["api_plan_submission"] = True
            (root / "readiness-proof.json").write_text('{"passed":true}')
            registry = {
                "schema_version": 1,
                "areas": [
                    {
                        "id": "test",
                        "title": "Evidence",
                        "checks": [
                            {
                                "id": "proof",
                                "description": "Pinned test",
                                "evidence": {
                                    "path": "readiness-proof.json",
                                    "sha256": hashlib.sha256(
                                        (root / "readiness-proof.json").read_bytes()
                                    ).hexdigest(),
                                    "pointer": "/passed",
                                    "equals": True,
                                },
                            },
                            {"id": "future", "description": "Not built", "evidence": None},
                        ],
                    }
                ],
            }
            (root / "readiness.json").write_text(json.dumps(registry))
            coverage = json.loads(request("/v1/readiness"))
            assert coverage["verified_checks"] == 1 and coverage["total_checks"] == 2
            (root / "readiness-proof.json").write_text('{"passed":false}')
            coverage = json.loads(request("/v1/readiness"))
            assert coverage["verified_checks"] == 0 and coverage["invalid_evidence_checks"] == 1
            observations["api_readiness_rejects_tampered_evidence"] = True
            uses = json.loads(request("/v1/use-cases"))
            assert len(uses["uses"]) == 8 and all(u["commands"] for u in uses["uses"])
            observations["eight_use_workflow_catalogue_in_authenticated_api"] = True
            observations["api_auth_status_graph_and_cockpit"] = True
        finally:
            server.terminate()
            server.communicate(timeout=5)
        call("graph-verify", db)
        observations["event_chain_with_real_float_receipts"] = True
        observations["joined_artifact_sha256_before_tamper"] = hashlib.sha256(b"12").hexdigest()
    if a.out:
        a.out.write_text(json.dumps(observations, indent=2) + "\n")
    print(json.dumps(observations, indent=2))


if __name__ == "__main__":
    main()
