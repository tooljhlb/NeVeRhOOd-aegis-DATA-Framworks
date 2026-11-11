"""Verify a returned worker export against a separately supplied commit identity."""

import argparse
import hashlib
import json
from pathlib import Path


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def verify(root, expected_revision):
    receipt = json.loads((root / "worker-receipt.json").read_text())
    manifest_path = root / "manifest.json"
    if receipt["source_revision"] != expected_revision:
        raise ValueError("worker source revision mismatch")
    if receipt["export_manifest_sha256"] != digest(manifest_path):
        raise ValueError("worker manifest changed")
    manifest = json.loads(manifest_path.read_text())
    required = {
        "checkpoint.pt",
        "acceptance.json",
        "comparison.json",
        "sources.json",
        "dataset-manifest.json",
        "training-receipt.json",
    }
    if set(manifest["files"]) != required:
        raise ValueError("unexpected export membership")
    for name, sha in manifest["files"].items():
        path = root / name
        if path.is_symlink() or not path.is_file() or digest(path) != sha:
            raise ValueError("returned artifact mismatch: " + name)
    decision = json.loads((root / "acceptance.json").read_text())
    comparison = json.loads((root / "comparison.json").read_text())
    training = json.loads((root / "training-receipt.json").read_text())
    checkpoint = manifest["files"]["checkpoint.pt"]
    if (
        decision.get("accepted") is not True
        or comparison["candidate"]["checkpoint_sha256"] != checkpoint
        or training["checkpoint_sha256"] != checkpoint
    ):
        raise ValueError("returned checkpoint not bound to accepted evaluation")
    return {
        "verified": True,
        "source_revision": expected_revision,
        "run_id": manifest["run_id"],
        "checkpoint_sha256": checkpoint,
        "scope": "transport and evidence consistency; worker is trusted, not independently attested",
    }


def main():
    p = argparse.ArgumentParser()
    p.add_argument("root", type=Path)
    p.add_argument("--expected-revision", required=True)
    p.add_argument("--out", type=Path)
    a = p.parse_args()
    result = verify(a.root, a.expected_revision)
    raw = json.dumps(result, indent=2) + "\n"
    if a.out:
        a.out.write_text(raw)
    print(raw)


if __name__ == "__main__":
    main()
