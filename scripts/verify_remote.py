"""Native HTTP worker integration with disjoint controller/worker directories."""

import argparse
import hashlib
import json
import os
import secrets
import socket
import subprocess
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path


def main():
    p = argparse.ArgumentParser()
    p.add_argument("binary", type=Path)
    p.add_argument("--out", type=Path)
    p.add_argument("--image", help="Optional locally provisioned digest-pinned OCI image")
    a = p.parse_args()
    binary = str(a.binary.resolve())
    observations = {
        "binary_sha256": hashlib.sha256(Path(binary).read_bytes()).hexdigest(),
        "verification_code_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
    }
    children = []
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        controller_root = root / "controller"
        controller_root.mkdir()
        worker_root = root / "workers"
        bundle = controller_root / "bundle"
        bundle.mkdir()
        data = b"real transfer\n" * 170000
        (bundle / "source.bin").write_bytes(data)
        db = controller_root / "queue.db"
        tokens = {"alpha": secrets.token_urlsafe(32), "beta": secrets.token_urlsafe(32)}
        registry = controller_root / "tokens.json"
        registry.write_text(json.dumps(tokens))
        registry.chmod(0o600)
        with socket.socket() as sock:
            sock.bind(("127.0.0.1", 0))
            port = sock.getsockname()[1]
        base = f"http://127.0.0.1:{port}"

        def call(*args, ok=True):
            r = subprocess.run(
                [binary, *map(str, args)], capture_output=True, text=True, timeout=45
            )
            if ok and r.returncode:
                raise RuntimeError(r.stderr + r.stdout)
            return r

        def request(path, value=None, worker="alpha", method="POST", raw=None, token=None):
            payload = (
                raw if raw is not None else json.dumps(value).encode() if value is not None else b""
            )
            headers = {
                "Authorization": "Bearer " + (token if token is not None else tokens[worker]),
                "X-Aegis-Worker": worker,
                "Content-Type": "application/octet-stream"
                if raw is not None
                else "application/json",
            }
            req = urllib.request.Request(
                base + path,
                data=None if method == "GET" else payload,
                headers=headers,
                method=method,
            )
            try:
                with urllib.request.urlopen(req, timeout=10) as response:
                    return response.status, response.read()
            except urllib.error.HTTPError as e:
                return e.code, e.read()

        def start_server():
            process = subprocess.Popen(
                [
                    binary,
                    "worker-controller",
                    str(db),
                    "--tokens",
                    str(registry),
                    "--bind",
                    f"127.0.0.1:{port}",
                    "--ttl",
                    "6",
                ],
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
            )
            children.append(process)
            deadline = time.monotonic() + 10
            while time.monotonic() < deadline:
                try:
                    with urllib.request.urlopen(base + "/health", timeout=0.5):
                        return process
                except (OSError, urllib.error.URLError):
                    if process.poll() is not None:
                        raise RuntimeError(process.communicate()[1].decode())
                    time.sleep(0.05)
            raise AssertionError("controller did not start")

        def enqueue(job, sleep=0, isolated=False):
            plan = dict(
                run_id=job,
                workspace=".",
                mode="bounded",
                max_slots=1,
                max_wall_seconds=40,
                allowed_operations=["test"],
                stages=[
                    dict(
                        id="task",
                        operation="test",
                        program="/bin/sh",
                        args=["-c", f"touch started; sleep {sleep}; cat source.bin > result.bin"],
                        inputs=["source.bin"],
                        outputs=["result.bin"],
                        timeout_seconds=30,
                    )
                ],
            )
            if isolated:
                plan["stages"][0]["container"] = dict(
                    image=a.image, memory_mib=128, cpus=1, pids=32
                )
            path = controller_root / "plan.json"
            path.write_text(json.dumps(plan))
            call("worker-enqueue", db, path, bundle)

        def client(job, trusted=True):
            env = dict(os.environ, AEGIS_WORKER_TOKEN=tokens["alpha"])
            args = [binary, "worker-remote", base, "alpha", str(worker_root / job)]
            if trusted:
                args.append("--trusted-local")
            process = subprocess.Popen(
                args, env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True
            )
            children.append(process)
            return process

        def wait_started(job):
            deadline = time.monotonic() + 12
            while time.monotonic() < deadline:
                if list((worker_root / job).glob("**/started")):
                    return
                time.sleep(0.03)
            raise AssertionError("remote worker did not start")

        try:
            server = start_server()
            assert request("/v1/claim", token="bad")[0] == 401
            observations["worker_authentication_required"] = True
            enqueue("roundtrip")
            process = client("roundtrip")
            stdout, stderr = process.communicate(timeout=30)
            assert process.returncode == 0, stderr + stdout
            result = json.loads(stdout)
            assert result["transport_verified"]
            assert (Path(result["root"]) / "result.bin").read_bytes() == data
            assert json.loads(call("remote-verify", db, "roundtrip").stdout)["verified"]
            observations["multi_chunk_input_output_native_roundtrip"] = True
            observations["worker_uses_disjoint_workspace"] = True
            enqueue("isolation-required")
            process = client("isolation-required", trusted=False)
            process.communicate(timeout=15)
            assert process.returncode != 0
            observations["host_execution_requires_explicit_trust"] = True

            if a.image:
                enqueue("oci-roundtrip", isolated=True)
                process = client("oci-roundtrip", trusted=False)
                stdout, stderr = process.communicate(timeout=40)
                assert process.returncode == 0, stderr + stdout
                assert json.loads(stdout)["transport_verified"]
                call("remote-verify", db, "oci-roundtrip")
                observations["native_remote_execution_with_oci_required"] = True

            enqueue("fencing")
            code, body = request("/v1/claim")
            assert code == 200
            first = json.loads(body)["lease"]
            assert request("/v1/renew", first, worker="beta")[0] == 409
            time.sleep(6.2)
            code, body = request("/v1/claim", worker="beta")
            second = json.loads(body)["lease"]
            assert code == 200 and second["epoch"] == first["epoch"] + 1
            assert request("/v1/renew", first)[0] == 409
            query = dict(id=first["id"], epoch=first["epoch"], path="result.bin", offset=0)
            assert request("/v1/upload?" + urllib.parse.urlencode(query), raw=b"stale")[0] == 409
            query.update(epoch=second["epoch"])
            upload = "/v1/upload?" + urllib.parse.urlencode(query)
            assert request(upload, worker="beta", raw=b"verified")[0] == 200
            assert request(upload, worker="beta", raw=b"verified")[0] == 200
            assert request(upload, worker="beta", raw=b"conflict")[0] == 409
            bad_path = dict(query, path="../escape")
            assert (
                request(
                    "/v1/upload?" + urllib.parse.urlencode(bad_path), worker="beta", raw=b"bad"
                )[0]
                == 409
            )
            assert request(upload, worker="beta", raw=b"x" * (1024 * 1024 + 1))[0] == 413
            completion = dict(lease=second, outputs={"result.bin": "0" * 64})
            assert request("/v1/complete", completion, worker="beta")[0] == 409
            completion["outputs"]["result.bin"] = hashlib.sha256(b"verified").hexdigest()
            code, body = request("/v1/complete", completion, worker="beta")
            assert code == 200, body
            result = json.loads(body)
            assert request("/v1/complete", completion, worker="beta")[0] == 200
            (Path(result["root"]) / "result.bin").write_bytes(b"tampered")
            assert call("remote-verify", db, "fencing", ok=False).returncode != 0
            observations.update(
                expired_lease_reassigned=True,
                cross_worker_renewal_rejected=True,
                stale_upload_rejected=True,
                retransmission_idempotent=True,
                conflicting_retransmission_rejected=True,
                traversal_rejected=True,
                oversized_chunk_rejected=True,
                hash_mismatch_rejected=True,
                duplicate_completion_idempotent=True,
                post_return_tamper_rejected=True,
            )

            enqueue("disconnect", sleep=3)
            process = client("disconnect")
            wait_started("disconnect")
            server.kill()
            server.communicate(timeout=5)
            process.communicate(timeout=10)
            assert process.returncode != 0
            server = start_server()
            time.sleep(6.2)
            replacement = client("disconnect-replacement")
            stdout, stderr = replacement.communicate(timeout=30)
            assert replacement.returncode == 0, stderr + stdout
            assert json.loads(stdout)["epoch"] == 2
            call("remote-verify", db, "disconnect")
            observations["controller_disconnect_stops_execution_and_reclaims_epoch"] = True

            enqueue("cancel", sleep=20)
            process = client("cancel")
            wait_started("cancel")
            call("cancel", db, "cancel")
            process.communicate(timeout=10)
            assert process.returncode != 0
            assert call("remote-verify", db, "cancel", ok=False).returncode != 0
            observations["remote_cancellation_stops_without_promotion"] = True
        finally:
            for process in children:
                if process.poll() is None:
                    process.kill()
                process.communicate(timeout=10)
    raw = json.dumps(observations, indent=2) + "\n"
    if a.out:
        a.out.write_text(raw)
    print(raw)


if __name__ == "__main__":
    main()
