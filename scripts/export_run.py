"""Export only a locally accepted, hash-matched checkpoint plus evidence."""

import argparse
import hashlib
import json
import shutil
from pathlib import Path


def digest(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for block in iter(lambda: f.read(1024 * 1024), b""):
            h.update(block)
    return h.hexdigest()


def main():
    p = argparse.ArgumentParser()
    p.add_argument("run", type=Path)
    p.add_argument("--run-id", required=True)
    a = p.parse_args()
    root = a.run
    decision = json.loads((root / "acceptance.json").read_text())
    if decision.get("accepted") is not True:
        raise ValueError("run was not accepted")
    gates = root / "gates.json"
    if decision["gate_config_sha256"] != digest(gates):
        raise ValueError("acceptance configuration changed")
    for gate in json.loads(gates.read_text()):
        if digest(gate["evidence_file"]) != gate["evidence_sha256"]:
            raise ValueError("acceptance evidence changed")
    comparison = json.loads((root / "comparison.json").read_text())
    checkpoint = root / "continuous/checkpoint.pt"
    if digest(checkpoint) != comparison["candidate"]["checkpoint_sha256"]:
        raise ValueError("checkpoint differs from evaluated model")
    out = root / "export"
    out.mkdir(exist_ok=False)
    manifest = dict(
        schema_version=1, run_id=a.run_id, status="locally_accepted_engineering_artifact", files={}
    )
    for name, path in {
        "checkpoint.pt": checkpoint,
        "acceptance.json": root / "acceptance.json",
        "comparison.json": root / "comparison.json",
        "sources.json": root / "sources.json",
        "dataset-manifest.json": root / "prepared/manifest.json",
        "training-receipt.json": root / "continuous/receipt.json",
    }.items():
        shutil.copyfile(path, out / name)
        if digest(path) != digest(out / name):
            raise ValueError("copy integrity failure")
        manifest["files"][name] = digest(out / name)
    (out / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")


if __name__ == "__main__":
    main()
