"""Executable multi-use cases; optional real OCI remote repair and contained drills."""

import argparse
import hashlib
import json
import os
import secrets
import shutil
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
    p.add_argument("--image")
    p.add_argument("--drills", action="store_true")
    a = p.parse_args()
    binary = a.binary.resolve()
    repo = Path(__file__).resolve().parents[1]
    observations = {
        "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
        "verification_code_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
    }
    with tempfile.TemporaryDirectory() as directory:
        root = Path(directory)
        source = root / "project"
        shutil.copytree(repo / "fixtures/maintenance-code", source)
        original = (source / "sum.sh").read_bytes()

        def call(*args, ok=True, env=None, timeout=180):
            r = subprocess.run(
                [str(binary), *map(str, args)],
                capture_output=True,
                text=True,
                env=env,
                timeout=timeout,
            )
            if ok and r.returncode:
                raise RuntimeError(r.stderr + r.stdout)
            return r

        def write(path, obj):
            path.write_text(json.dumps(obj))
            return path

        def spec(job, purpose="repair", required=False):
            return dict(
                schema_version=1,
                id=job,
                purpose=purpose,
                files=["sum.sh", "test.sh"],
                editable=["sum.sh"],
                protected=["test.sh"],
                checks=[
                    dict(id="semantic-tests", kind="test", program="/bin/sh", args=["test.sh"])
                ],
                baseline_exit_codes={"semantic-tests": 1 if purpose == "repair" else 0},
                timeout_seconds=30,
                require_remote=required,
            )

        patch = dict(edits=[dict(path="sum.sh", before="$1 - $2", after="$1 + $2", occurrences=1)])
        patch_path = write(root / "patch.json", patch)
        case = root / "case"
        call("case-capture", source, write(root / "spec.json", spec("semantic-repair")), case)
        generator = write(
            root / "generator.json",
            dict(
                id="fixture-proposer",
                kind="proposal",
                program=sys.executable,
                args=[str(repo / "fixtures/maintenance-proposer.py")],
            ),
        )
        proposal = json.loads(call("case-propose", case, generator, "--trusted-local").stdout)
        assert proposal["source_modified"] is False
        rehearsal = json.loads(
            call("case-rehearse", case, case / "proposal/patch.json", "--trusted-local").stdout
        )
        assert rehearsal["accepted"] and rehearsal["report"]["failure_reproduced"]
        assert (source / "sum.sh").read_bytes() == original
        observations["semantic_failure_reproduced_and_candidate_tests_pass"] = True
        observations["executable_proposal_adapter_without_source_mutation"] = True
        report_sha = rehearsal["report_sha256"]
        package = root / "exchange"
        exported = json.loads(call("case-export", case, package).stdout)
        replay = json.loads(
            call(
                "case-replay",
                package,
                root / "replay",
                "--exchange-sha256",
                exported["exchange_sha256"],
                "--trusted-local",
            ).stdout
        )
        assert replay["accepted"]
        observations["portable_bundle_independently_replayed"] = True
        assert call(
            "case-replay",
            package,
            root / "wrong-replay",
            "--exchange-sha256",
            "0" * 64,
            "--trusted-local",
            ok=False,
        ).returncode
        (package / "bundle/sum.sh").write_text("tampered")
        assert call(
            "case-replay",
            package,
            root / "tamper-replay",
            "--exchange-sha256",
            exported["exchange_sha256"],
            "--trusted-local",
            ok=False,
        ).returncode
        observations["exchange_tamper_rejected"] = True
        bad_case = root / "protected-case"
        call("case-capture", source, root / "spec.json", bad_case)
        bad_patch = write(
            root / "bad-patch.json",
            dict(edits=[dict(path="test.sh", before="-eq 5", after="-eq -1", occurrences=1)]),
        )
        assert call("case-rehearse", bad_case, bad_patch, "--trusted-local", ok=False).returncode
        observations["protected_test_edit_rejected"] = True
        false_spec = spec("false-baseline")
        false_spec["baseline_exit_codes"]["semantic-tests"] = 0
        false_case = root / "false-case"
        call("case-capture", source, write(root / "false-spec.json", false_spec), false_case)
        assert call("case-rehearse", false_case, patch_path, "--trusted-local", ok=False).returncode
        observations["baseline_mismatch_rejects_promotion"] = True
        lineage = root / "lineage.db"
        call(
            "lineage-link",
            lineage,
            case / "report.json",
            source / "sum.sh",
            source / "test.sh",
            case / "patch.json",
        )
        call("case-promote", case, source, "--report-sha256", report_sha, "--lineage-db", lineage)
        assert (source / "sum.sh").read_bytes() != original
        call("case-promote", case, source, "--report-sha256", report_sha, "--rollback")
        assert (source / "sum.sh").read_bytes() == original
        observations["review_hash_bound_promotion_and_rollback"] = True
        source_sha = hashlib.sha256(original).hexdigest()
        impact = json.loads(
            call("lineage-quarantine", lineage, source_sha, "Source correction required").stdout
        )
        assert "artifact:" + report_sha in impact["potentially_affected"]
        assert call(
            "case-promote",
            case,
            source,
            "--report-sha256",
            report_sha,
            "--lineage-db",
            lineage,
            ok=False,
        ).returncode
        observations["source_quarantine_blocks_dependent_promotion"] = True
        (source / "sum.sh").write_bytes(original + b"# concurrent change\n")
        assert call(
            "case-promote", case, source, "--report-sha256", report_sha, ok=False
        ).returncode
        (source / "sum.sh").write_bytes(original)
        observations["changed_project_preimage_rejected"] = True
        upgrade_source = root / "upgrade-project"
        shutil.copytree(source, upgrade_source)
        (upgrade_source / "sum.sh").write_text(original.decode().replace("$1 - $2", "$1 + $2"))
        upgrade = root / "upgrade-case"
        call(
            "case-capture",
            upgrade_source,
            write(root / "upgrade-spec.json", spec("upgrade", purpose="upgrade")),
            upgrade,
        )
        upgrade_patch = write(
            root / "upgrade-patch.json",
            dict(edits=[dict(path="sum.sh", before="'%s\\n'", after="'%d\\n'", occurrences=1)]),
        )
        assert json.loads(call("case-rehearse", upgrade, upgrade_patch, "--trusted-local").stdout)[
            "accepted"
        ]
        observations["upgrade_rehearsal_passes_same_checks_before_after"] = True
        dataset = root / "dataset.jsonl"
        rows = [dict(id=str(i), group="g" + str(i // 2), expected=i % 2) for i in range(6)]
        dataset.write_text("\n".join(map(json.dumps, rows)) + "\n")
        baseline = root / "baseline.jsonl"
        baseline.write_text(
            "\n".join(json.dumps(dict(id=r["id"], prediction=0)) for r in rows) + "\n"
        )
        adapter = write(
            root / "adapter.json",
            dict(
                id="prediction-contract-fixture",
                kind="predict",
                program=sys.executable,
                args=[
                    "-c",
                    "import sys,json; from pathlib import Path; rows=[json.loads(x) for x in Path(sys.argv[1]).read_text().splitlines()]; Path(sys.argv[2]).write_text(''.join(json.dumps(dict(id=r['id'],prediction=r['expected']))+'\\n' for r in rows))",
                    "{dataset}",
                    "{predictions}",
                ],
            ),
        )
        predictions = root / "predictions"
        call("lab-run", dataset, adapter, predictions, "--trusted-local")
        comparison = json.loads(
            call(
                "lab-compare",
                dataset,
                baseline,
                predictions / "predictions.jsonl",
                "--out",
                root / "comparison.json",
            ).stdout
        )
        assert (
            comparison["candidate"] == 1
            and comparison["baseline"] == 0.5
            and comparison["groups"] == 3
        )
        observations["prediction_adapter_and_paired_group_comparison"] = True
        with (predictions / "predictions.jsonl").open("a") as stream:
            stream.write(json.dumps(dict(id="unknown", prediction=1)) + "\n")
        assert call(
            "lab-compare",
            dataset,
            baseline,
            predictions / "predictions.jsonl",
            "--out",
            root / "invalid-comparison.json",
            ok=False,
        ).returncode
        observations["prediction_coverage_mismatch_rejected"] = True
        regression_data = root / "regression.jsonl"
        regression_data.write_text(
            "".join(
                json.dumps(dict(id=str(i), group="only-group", expected=y)) + "\n"
                for i, y in enumerate([1, 3])
            )
        )
        regression_base = root / "regression-base.jsonl"
        regression_base.write_text(
            "".join(json.dumps(dict(id=str(i), prediction=0)) + "\n" for i in range(2))
        )
        regression_candidate = root / "regression-candidate.jsonl"
        regression_candidate.write_text(
            "".join(json.dumps(dict(id=str(i), prediction=y)) + "\n" for i, y in enumerate([1, 3]))
        )
        regression = json.loads(
            call(
                "lab-compare",
                regression_data,
                regression_base,
                regression_candidate,
                "--mode",
                "regression",
                "--out",
                root / "regression-result.json",
            ).stdout
        )
        assert (
            regression["baseline"] == 5
            and regression["candidate"] == 0
            and regression["paired_group_bootstrap_ci95"] is None
        )
        observations["regression_mse_and_insufficient_group_uncertainty"] = True
        now = int(time.time())
        request = write(
            root / "request.json",
            dict(
                cpu=1,
                memory_mib=128,
                gpu_count=0,
                gpu_memory_mib=0,
                duration_seconds=10,
                max_cost_micros=100,
                capabilities=["shell"],
                max_age_seconds=60,
            ),
        )
        workers = [
            dict(
                id="alpha",
                observed_at_unix=now,
                cpu_available=2,
                memory_mib_available=512,
                gpu_count_available=0,
                gpu_memory_mib_available=0,
                price_micros_per_second=2,
                capabilities=["shell"],
                images=[],
            ),
            dict(
                id="stale",
                observed_at_unix=now - 1000,
                cpu_available=2,
                memory_mib_available=512,
                gpu_count_available=0,
                gpu_memory_mib_available=0,
                price_micros_per_second=1,
                capabilities=["shell"],
                images=[],
            ),
        ]
        quote = json.loads(
            call(
                "broker-quote",
                request,
                write(root / "inventory.json", workers),
                "--out",
                root / "quote.json",
            ).stdout
        )
        assert quote["chosen"]["worker"] == "alpha" and quote["rejected"][0]["reasons"] == [
            "stale_inventory"
        ]
        observations["freshness_resource_capability_cost_admission_quote"] = True
        if a.drills:
            drill = json.loads(
                call(
                    "incident-drill",
                    repo,
                    binary,
                    root / "incident",
                    "--scenario",
                    "worker-loss",
                    "--python",
                    sys.executable,
                    "--trusted-local",
                ).stdout
            )
            assert drill["observations"]["worker_sigkill_reclaimed_in_fresh_epoch"]
            observations["contained_incident_drill_with_actual_worker_kill"] = True
        else:
            observations["contained_incident_drill"] = "not_run_without_drills_option"
        if a.image:
            remote_case = root / "remote-case"
            call(
                "case-capture",
                source,
                write(root / "remote-spec.json", spec("remote-repair", required=True)),
                remote_case,
            )
            rr = json.loads(
                call("case-rehearse", remote_case, patch_path, "--trusted-local").stdout
            )
            assert call(
                "case-promote",
                remote_case,
                source,
                "--report-sha256",
                rr["report_sha256"],
                ok=False,
            ).returncode
            remote_bundle = root / "remote-bundle"
            remote_bundle.mkdir()
            for name in ["sum.sh", "test.sh"]:
                shutil.copyfile(remote_case / "candidate" / name, remote_bundle / name)
            db = root / "remote.db"
            plan = dict(
                run_id="remote-repair",
                workspace=".",
                mode="bounded",
                max_slots=1,
                max_wall_seconds=60,
                allowed_operations=["test"],
                stages=[
                    dict(
                        id="semantic-tests",
                        operation="test",
                        program="/bin/sh",
                        args=["test.sh"],
                        inputs=["sum.sh", "test.sh"],
                        outputs=["verified.json"],
                        timeout_seconds=30,
                        container=dict(image=a.image, memory_mib=128, cpus=1, pids=32),
                    )
                ],
            )
            call(
                "worker-enqueue",
                db,
                write(root / "remote-plan.json", plan),
                remote_bundle,
                "--worker",
                "alpha",
            )
            token = secrets.token_urlsafe(32)
            registry = write(
                root / "tokens.json", dict(alpha=token, beta=secrets.token_urlsafe(32))
            )
            registry.chmod(0o600)
            with socket.socket() as sock:
                sock.bind(("127.0.0.1", 0))
                port = sock.getsockname()[1]
            base = f"http://127.0.0.1:{port}"
            server = subprocess.Popen(
                [
                    str(binary),
                    "worker-controller",
                    str(db),
                    "--tokens",
                    str(registry),
                    "--bind",
                    f"127.0.0.1:{port}",
                ],
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
            )
            try:
                for _ in range(100):
                    try:
                        with urllib.request.urlopen(base + "/health", timeout=0.5):
                            break
                    except (OSError, urllib.error.URLError):
                        time.sleep(0.05)
                wrong = urllib.request.Request(
                    base + "/v1/claim",
                    data=b"",
                    headers={
                        "Authorization": "Bearer " + json.loads(registry.read_text())["beta"],
                        "X-Aegis-Worker": "beta",
                    },
                    method="POST",
                )
                with urllib.request.urlopen(wrong, timeout=5) as response:
                    assert json.loads(response.read())["state"] == "idle"
                call(
                    "worker-remote",
                    base,
                    "alpha",
                    root / "remote-worker",
                    env=dict(os.environ, AEGIS_WORKER_TOKEN=token),
                )
                remote_acceptance = json.loads(
                    call("case-accept-remote", remote_case, db, "remote-repair").stdout
                )
                assert remote_acceptance["accepted"]
                remote_lineage = root / "remote-lineage.db"
                call(
                    "lineage-link",
                    remote_lineage,
                    remote_case / "report.json",
                    source / "sum.sh",
                    source / "test.sh",
                    remote_case / "patch.json",
                )
                call(
                    "case-promote",
                    remote_case,
                    source,
                    "--report-sha256",
                    rr["report_sha256"],
                    "--remote-db",
                    db,
                    "--remote-job",
                    "remote-repair",
                    "--lineage-db",
                    remote_lineage,
                )
                assert b"$1 + $2" in (source / "sum.sh").read_bytes()
                observations["remote_isolated_semantic_repair_and_policy_gated_promotion"] = True
                observations["chosen_worker_identity_enforced_at_claim"] = True
            finally:
                server.terminate()
                server.communicate(timeout=10)
        else:
            observations["remote_isolated_semantic_repair"] = "not_run_no_OCI_image"
    raw = json.dumps(observations, indent=2) + "\n"
    if a.out:
        a.out.write_text(raw)
    print(raw)


if __name__ == "__main__":
    main()
