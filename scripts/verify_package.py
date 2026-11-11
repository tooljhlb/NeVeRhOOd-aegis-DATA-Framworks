"""Test fresh installation, tamper rejection and a real repository C++ workflow."""

import argparse
import hashlib
import json
import shutil
import sqlite3
import subprocess
import tempfile
from pathlib import Path

p = argparse.ArgumentParser()
p.add_argument("binary", type=Path)
p.add_argument("--revision", required=True)
p.add_argument("--out", type=Path, required=True)
a = p.parse_args()
repo = Path(__file__).resolve().parents[1]
with tempfile.TemporaryDirectory() as tmp:
    root = Path(tmp)
    package, prefix = root / "package", root / "installed"

    def call(*args, ok=True):
        result = subprocess.run(list(map(str, args)), capture_output=True, text=True, timeout=120)
        assert (result.returncode == 0) == ok, result.stdout + result.stderr
        return result

    call("bash", repo / "scripts/package.sh", a.binary.resolve(), package, a.revision)
    call("bash", package / "install.sh", prefix)
    binary = prefix / "bin/aegis-data"
    call("bash", package / "install.sh", prefix, ok=False)
    with (package / "aegis-data").open("ab") as f:
        f.write(b"tampered")
    call("bash", package / "install.sh", root / "tampered-install", ok=False)
    assert not (root / "tampered-install").exists()
    project = root / "project"
    (project / "native/tests").mkdir(parents=True)
    for name in ["native/reference.hpp", "native/tests/reference_test.cpp"]:
        shutil.copyfile(repo / name, project / name)
    source = project / "native/tests/reference_test.cpp"
    original = source.read_bytes()
    plan = {
        "run_id": "installed-project",
        "workspace": str(project),
        "mode": "bounded",
        "max_slots": 1,
        "max_wall_seconds": 120,
        "allowed_operations": ["build", "test"],
        "stages": [
            {
                "id": "build",
                "operation": "build",
                "program": "c++",
                "args": ["-std=c++17", "native/tests/reference_test.cpp", "-o", "reference-test"],
                "inputs": ["native/reference.hpp", "native/tests/reference_test.cpp"],
                "outputs": ["reference-test"],
                "timeout_seconds": 60,
            },
            {
                "id": "test",
                "operation": "test",
                "program": "./reference-test",
                "args": [],
                "depends_on": ["build"],
                "inputs": ["reference-test"],
                "outputs": [],
                "timeout_seconds": 20,
            },
        ],
    }
    profile = {
        "schema_version": 1,
        "id": "cpp-project",
        "require_container": False,
        "allowed_programs": ["c++", "./reference-test"],
        "allowed_images": [],
        "max_slots": 1,
        "max_wall_seconds": 120,
        "max_attempts": 1,
        "max_stage_seconds": 60,
        "max_cpu": 1,
        "max_memory_mib": 128,
    }
    config, policy, db, export = (
        root / "plan.json",
        root / "profile.json",
        root / "run.db",
        root / "export",
    )
    config.write_text(json.dumps(plan))
    policy.write_text(json.dumps(profile))
    # Doctor identifies build executable; project-generated test binary does not exist yet.
    call(binary, "project-doctor", config, ok=False)
    source.write_bytes(original + b"\n#error intentional workflow failure\n")
    call(binary, "project-run", policy, config, db, export, "--trusted-local", ok=False)
    assert not export.exists()
    source.write_bytes(original)
    # One attempt was consumed; a new run ID explicitly starts a corrected experiment.
    plan["run_id"] = "installed-project-recovered"
    config.write_text(json.dumps(plan))
    receipt = json.loads(
        call(binary, "project-run", policy, config, db, export, "--trusted-local").stdout
    )
    call(binary, "project-doctor", config)
    manifest = json.loads((export / "manifest.json").read_text())
    assert (
        hashlib.sha256((export / "manifest.json").read_bytes()).hexdigest()
        == receipt["manifest_sha256"]
    )
    for name, digest in manifest["workspace_files"].items():
        assert hashlib.sha256((export / "workspace" / name).read_bytes()).hexdigest() == digest
    assert manifest["log_files"] and (export / "events.json").exists()
    call(binary, "project-verify", export, "--manifest-sha256", receipt["manifest_sha256"])
    (export / "unexpected.txt").write_text("unexpected")
    call(
        binary, "project-verify", export, "--manifest-sha256", receipt["manifest_sha256"], ok=False
    )
    (export / "unexpected.txt").unlink()
    (export / "workspace/native/reference.hpp").write_text("tampered")
    call(
        binary, "project-verify", export, "--manifest-sha256", receipt["manifest_sha256"], ok=False
    )
    with sqlite3.connect(db) as connection:
        raw = connection.execute(
            "SELECT receipt FROM operator_stages WHERE run_id=? AND id='build'", (plan["run_id"],)
        ).fetchone()[0]
        altered = json.loads(raw)
        altered["wall_seconds"] = 999
        connection.execute(
            "UPDATE operator_stages SET receipt=? WHERE run_id=? AND id='build'",
            (json.dumps(altered), plan["run_id"]),
        )
    call(binary, "project-export", db, plan["run_id"], root / "forged-export", ok=False)
    assert not (root / "forged-export").exists()
    with sqlite3.connect(db) as connection:
        connection.execute(
            "UPDATE operator_stages SET receipt=? WHERE run_id=? AND id='build'",
            (raw, plan["run_id"]),
        )
    with sqlite3.connect(db) as connection:
        raw_plan = connection.execute(
            "SELECT plan FROM operator_runs WHERE id=?", (plan["run_id"],)
        ).fetchone()[0]
        altered_plan = json.loads(raw_plan)
        altered_plan["max_wall_seconds"] = 121
        connection.execute(
            "UPDATE operator_runs SET plan=? WHERE id=?", (json.dumps(altered_plan), plan["run_id"])
        )
    call(binary, "project-export", db, plan["run_id"], root / "changed-plan-export", ok=False)
    with sqlite3.connect(db) as connection:
        connection.execute("UPDATE operator_runs SET plan=? WHERE id=?", (raw_plan, plan["run_id"]))
    source.write_bytes(original + b"\n// changed after verification\n")
    call(binary, "project-export", db, plan["run_id"], root / "stale-export", ok=False)
    assert not (root / "stale-export").exists()
report = {
    "fresh_installation": True,
    "existing_install_not_overwritten": True,
    "tampered_package_rejected": True,
    "real_repository_cpp_build_and_test": True,
    "failed_build_withholds_export": True,
    "corrected_new_run_succeeds": True,
    "export_hashes_verified": True,
    "post_run_source_change_rejected": True,
    "anchored_export_tamper_and_extra_file_rejected": True,
    "receipt_projection_tamper_rejected": True,
    "stored_plan_identity_change_rejected": True,
    "scope": "Linux native runtime; repository C++ component, not independent-user validation",
}
a.out.write_text(json.dumps(report, indent=2) + "\n")
print(json.dumps(report))
