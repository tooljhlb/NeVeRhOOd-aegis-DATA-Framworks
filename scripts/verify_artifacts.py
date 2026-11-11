"""Exercise resumable artifacts, >5 GiB manifests and practical outcome reports."""

import argparse
import json
import shutil
import subprocess
import tempfile
from pathlib import Path

p = argparse.ArgumentParser()
p.add_argument("binary", type=Path)
p.add_argument("--out", type=Path, required=True)
p.add_argument("--large", action="store_true")
a = p.parse_args()
binary = a.binary.resolve()
with tempfile.TemporaryDirectory() as tmp:
    root = Path(tmp)

    def call(*args, ok=True):
        r = subprocess.run(
            [str(binary), *map(str, args)], capture_output=True, text=True, timeout=240
        )
        assert (r.returncode == 0) == ok, r.stdout + r.stderr
        return json.loads(r.stdout) if ok else None

    source = root / "source.bin"
    source.write_bytes(b"a" * (1024 * 1024) + b"b" * (1024 * 1024) + b"last")
    store = root / "source-store"
    dest = root / "destination-store"
    manifest = root / "manifest.json"
    packed = call("artifact-pack", source, store, manifest, "--chunk-mib", 1)
    anchor = packed["manifest_sha256"]
    assert call("artifact-pack", source, store, manifest, "--chunk-mib", 1)["new_chunks"] == 0
    m = json.loads(manifest.read_text())
    dest.mkdir()
    shutil.copyfile(store / m["chunks"][0], dest / m["chunks"][0])
    result = call("artifact-transfer", manifest, store, dest, "--manifest-sha256", anchor)
    assert result["reused_chunks"] == 1 and result["transferred_chunks"] == 2
    assert (
        call("artifact-transfer", manifest, store, dest, "--manifest-sha256", anchor)[
            "transferred_chunks"
        ]
        == 0
    )
    output = root / "body.bin"
    # An interrupted write left a verified full chunk and incomplete next chunk.
    (root / "body.bin.partial").write_bytes(source.read_bytes()[: 1024 * 1024 + 17])
    assert (
        call("artifact-materialize", manifest, dest, output, "--manifest-sha256", anchor)[
            "resumed_bytes"
        ]
        == 1024 * 1024
    )
    assert output.read_bytes() == source.read_bytes()
    assert call("artifact-materialize", manifest, dest, output, "--manifest-sha256", anchor)[
        "reused"
    ]
    (dest / m["chunks"][1]).write_bytes(b"corrupted")
    call("artifact-transfer", manifest, store, dest, "--manifest-sha256", anchor, ok=False)
    call(
        "artifact-materialize",
        manifest,
        store,
        root / "wrong.bin",
        "--manifest-sha256",
        "0" * 64,
        ok=False,
    )
    protected = root / "protected.bin"
    protected.write_bytes(b"untouched")
    (root / "symlink.bin.partial").symlink_to(protected)
    call(
        "artifact-materialize",
        manifest,
        store,
        root / "symlink.bin",
        "--manifest-sha256",
        anchor,
        ok=False,
    )
    assert protected.read_bytes() == b"untouched"
    if a.large:
        # Sparse input exercises all logical bytes while dedup keeps physical storage small.
        huge = root / "large-shard.bin"
        with huge.open("wb") as f:
            f.truncate(5 * 1024**3 + 123)
        huge_manifest = root / "large.json"
        large = call("artifact-pack", huge, root / "large-store", huge_manifest, "--chunk-mib", 8)
        assert large["size"] == 5 * 1024**3 + 123 and large["chunks"] == 641
    observations = root / "outcomes.jsonl"
    observations.write_text(
        json.dumps(
            {
                "run_id": "one",
                "time_to_first_success_ms": 1000,
                "recovery_ms": 200,
                "unnecessary_reruns": 0,
                "patch_attempts": 4,
                "patch_regressions": 1,
                "operator_interventions": 2,
            }
        )
        + "\n"
        + json.dumps({"run_id": "two"})
        + "\n"
    )
    report = root / "outcome-report.json"
    outcome = call("outcome-report", observations, report)
    assert outcome["time_to_first_success_ms"]["samples"] == 1
    assert outcome["patch_regression_rate"] == 0.25 and outcome["patch_observed_runs"] == 1
    observations.write_text('{"run_id":"invalid","patch_attempts":1,"patch_regressions":2}\n')
    call("outcome-report", observations, report, ok=False)
receipt = {
    "resumable_chunk_transfer": True,
    "partial_reconstruction_resume": True,
    "corruption_and_manifest_anchor_rejected": True,
    "logical_5_gib_plus_shard_packed": True if a.large else "not_run_without_large_flag",
    "large_source_is_sparse_test_data": a.large,
    "partial_symlink_rejected": True,
    "outcome_counts_and_missing_values_verified": True,
    "scope": "filesystem stores; not independently provisioned remote hosts or checkpoint correctness",
}
a.out.write_text(json.dumps(receipt, indent=2) + "\n")
print(json.dumps(receipt))
