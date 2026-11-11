"""Real worker crash, lease heartbeat/fencing, cancellation and sealed-return checks."""

import argparse
import json
import subprocess
import tempfile
import time
from pathlib import Path


def main():
    p = argparse.ArgumentParser()
    p.add_argument("binary", type=Path)
    p.add_argument("--out", type=Path)
    a = p.parse_args()
    binary = str(a.binary.resolve())
    observations = {}
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        bundle = root / "bundle"
        bundle.mkdir()
        (bundle / "source.txt").write_text("immutable source")
        db = root / "workers.db"
        config = root / "plan.json"

        def call(*args, ok=True):
            r = subprocess.run(
                [binary, *map(str, args)], capture_output=True, text=True, timeout=30
            )
            if ok and r.returncode:
                raise RuntimeError(r.stderr + r.stdout)
            return r

        def status(job):
            return json.loads(call("worker-status", db, job).stdout)

        def enqueue(job, sleep=0):
            plan = dict(
                run_id=job,
                workspace=".",
                mode="bounded",
                max_slots=1,
                max_wall_seconds=30,
                allowed_operations=["test"],
                stages=[
                    dict(
                        id="task",
                        operation="test",
                        program="/bin/sh",
                        args=["-c", f"touch started; sleep {sleep}; cat source.txt > output.txt"],
                        inputs=["source.txt"],
                        outputs=["output.txt"],
                        timeout_seconds=20,
                    )
                ],
            )
            config.write_text(json.dumps(plan))
            call("worker-enqueue", db, config, bundle)
            return plan

        def worker(name):
            return subprocess.Popen(
                [binary, "worker-once", str(db), name, "--ttl", "3", "--trusted-local"],
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )

        def wait_start(job, epoch=1):
            marker = Path(str(db) + ".workers") / job / f"epoch-{epoch}/workspace/started"
            deadline = time.monotonic() + 10
            while not marker.exists() and time.monotonic() < deadline:
                time.sleep(0.03)
            assert marker.exists()

        enqueue("heartbeat", 4)
        process = worker("alive")
        try:
            wait_start("heartbeat")
            time.sleep(3.2)
            assert (
                json.loads(
                    call("worker-once", db, "competitor", "--ttl", 3, "--trusted-local").stdout
                )["state"]
                == "idle"
            )
            stdout, stderr = process.communicate(timeout=10)
            assert process.returncode == 0, stderr + stdout
        finally:
            if process.poll() is None:
                process.kill()
                process.communicate()
        assert json.loads(call("worker-verify", db, "heartbeat").stdout)["verified"]
        observations["heartbeat_prevents_reclaim_after_original_expiry"] = True
        observations["sealed_return_verified"] = True
        receipt = json.loads(status("heartbeat")["result"])
        sealed = json.loads(Path(receipt["receipt"]).read_text())
        (Path(sealed["workspace"]) / "output.txt").write_text("tampered")
        assert call("worker-verify", db, "heartbeat", ok=False).returncode != 0
        observations["returned_artifact_tamper_rejected"] = True

        enqueue("lost", 1)
        process = worker("lost-worker")
        wait_start("lost")
        epoch = status("lost")["epoch"]
        process.kill()
        process.communicate(timeout=5)
        time.sleep(3.2)
        r = json.loads(call("worker-once", db, "replacement", "--ttl", 3, "--trusted-local").stdout)
        assert r["epoch"] == epoch + 1 and r["state"] == "done"
        call("worker-verify", db, "lost")
        stale = call("complete", db, "lost", "lost-worker", epoch, "stale", ok=False)
        assert stale.returncode != 0
        observations["worker_sigkill_reclaimed_in_fresh_epoch"] = True
        observations["stale_completion_rejected"] = True

        enqueue("cancelled", 15)
        process = worker("cancel-worker")
        try:
            wait_start("cancelled")
            call("cancel", db, "cancelled")
            process.communicate(timeout=5)
            assert process.returncode != 0 and status("cancelled")["state"] == "cancelled"
            assert call("worker-verify", db, "cancelled", ok=False).returncode != 0
        finally:
            if process.poll() is None:
                process.kill()
                process.communicate()
        observations["cancellation_stops_worker_without_promoting_result"] = True
        enqueue("bundle-tamper")
        path = Path(str(db) + ".workers") / "bundle-tamper/bundle/source.txt"
        path.write_text("modified")
        assert (
            call("worker-once", db, "reader", "--ttl", 3, "--trusted-local", ok=False).returncode
            != 0
        )
        observations["queued_bundle_tamper_rejected"] = True
    raw = json.dumps(observations, indent=2) + "\n"
    if a.out:
        a.out.write_text(raw)
    print(raw)


if __name__ == "__main__":
    main()
