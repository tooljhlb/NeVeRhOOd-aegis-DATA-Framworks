"""Import official evaluator outputs with explicit provenance; never execute shell strings."""

import argparse
import hashlib
import json
from pathlib import Path


def main():
    p = argparse.ArgumentParser()
    p.add_argument(
        "--engine",
        required=True,
        choices=["VBench", "MM-Vet", "SWE-bench", "G-Eval", "FAD"],
    )
    p.add_argument("--result", required=True)
    p.add_argument("--revision", required=True)
    p.add_argument("--model-sha256", required=True)
    p.add_argument("--dataset-sha256", required=True)
    p.add_argument("--out", required=True)
    a = p.parse_args()
    source = Path(a.result)
    raw = source.read_bytes()
    payload = json.loads(raw)
    for v in (a.model_sha256, a.dataset_sha256):
        if len(v) != 64 or any(c not in "0123456789abcdef" for c in v):
            raise ValueError("SHA256 required")
    Path(a.out).write_text(
        json.dumps(
            {
                "engine": a.engine,
                "revision": a.revision,
                "model_sha256": a.model_sha256,
                "dataset_sha256": a.dataset_sha256,
                "result_sha256": hashlib.sha256(raw).hexdigest(),
                "status": "imported_unverified",
                "result": payload,
            },
            indent=2,
        )
    )


if __name__ == "__main__":
    main()
