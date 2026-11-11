"""Exercise actual pinned-image OCI isolation; fail if Docker is unavailable."""

import argparse
import json
import subprocess
import tempfile
from pathlib import Path


def main():
    p = argparse.ArgumentParser()
    p.add_argument("binary", type=Path)
    p.add_argument("--image", required=True)
    p.add_argument("--out", type=Path)
    a = p.parse_args()
    binary = str(a.binary.resolve())
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        workspace = root / "workspace"
        workspace.mkdir()
        (root / "host-private").write_text("must not be visible")
        checks = f"set -eu; test ! -e {root}/host-private; test ! -e /var/run/docker.sock; "
        checks += "if touch /etc/aegis-write-test 2>/dev/null; then exit 21; fi; "
        checks += 'test "$(cat /sys/fs/cgroup/memory.max)" = 134217728; '
        checks += 'test "$(cat /sys/fs/cgroup/pids.max)" = 32; '
        checks += "test \"$(cat /sys/fs/cgroup/cpu.max)\" = '100000 100000'; "
        checks += 'test "$(ls /sys/class/net)" = lo; printf verified > proof.txt'
        plan = dict(
            run_id="isolated-proof",
            workspace=str(workspace),
            mode="bounded",
            max_slots=1,
            max_wall_seconds=60,
            allowed_operations=["test"],
            stages=[
                dict(
                    id="isolated",
                    operation="test",
                    program="/bin/sh",
                    args=["-c", checks],
                    outputs=["proof.txt"],
                    timeout_seconds=30,
                    container=dict(image=a.image, memory_mib=128, cpus=1, pids=32),
                )
            ],
        )
        config = root / "plan.json"
        config.write_text(json.dumps(plan))
        db = root / "operator.db"
        r = subprocess.run(
            [binary, "operator-run", str(db), str(config), "--trusted-local"],
            capture_output=True,
            text=True,
            timeout=70,
        )
        if r.returncode:
            raise RuntimeError(r.stderr + r.stdout)
        assert json.loads(r.stdout)["status"] == "completed"
        assert (workspace / "proof.txt").read_text() == "verified"
        # Unpinned images must fail before executing anything.
        plan["run_id"] = "unpinned"
        plan["stages"][0]["container"]["image"] = "busybox:latest"
        config.write_text(json.dumps(plan))
        r = subprocess.run(
            [binary, "operator-run", str(db), str(config), "--trusted-local"],
            capture_output=True,
            timeout=10,
        )
        assert r.returncode != 0
    result = dict(
        actual_oci_execution=True,
        read_only_root=True,
        no_external_network=True,
        memory_limit_mib=128,
        cpu_limit=1,
        process_limit=32,
        host_daemon_socket_absent=True,
        workspace_output_verified=True,
        unpinned_image_rejected=True,
        image=a.image,
        scope="Linux OCI integration, not an escape-resistance certification",
    )
    raw = json.dumps(result, indent=2) + "\n"
    if a.out:
        a.out.write_text(raw)
    print(raw)


if __name__ == "__main__":
    main()
