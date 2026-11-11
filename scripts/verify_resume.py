"""Compare actual model and optimizer tensor state, not serialized file bytes."""

import argparse
import hashlib
import json
from pathlib import Path

import torch

p = argparse.ArgumentParser()
p.add_argument("continuous")
p.add_argument("resumed")
p.add_argument("--out")
a = p.parse_args()
x = torch.load(a.continuous, map_location="cpu", weights_only=True)
y = torch.load(a.resumed, map_location="cpu", weights_only=True)


def equal(x, y):
    if isinstance(x, torch.Tensor):
        return isinstance(y, torch.Tensor) and torch.equal(x, y)
    if isinstance(x, dict):
        return isinstance(y, dict) and x.keys() == y.keys() and all(equal(x[k], y[k]) for k in x)
    if isinstance(x, (list, tuple)):
        return type(x) is type(y) and len(x) == len(y) and all(equal(i, j) for i, j in zip(x, y))
    return x == y


assert equal(x, y), "checkpoint state mismatch"
print("PASS: model, optimizer, RNG and cursor state exactly match")

if a.out:
    Path(a.out).write_text(
        json.dumps(
            {
                "schema_version": 1,
                "verified": 1,
                "continuous_sha256": hashlib.sha256(Path(a.continuous).read_bytes()).hexdigest(),
                "resumed_sha256": hashlib.sha256(Path(a.resumed).read_bytes()).hexdigest(),
            },
            indent=2,
        )
        + "\n"
    )
