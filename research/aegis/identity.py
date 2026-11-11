"""Content-addressed computation identity. Legacy unidentified resume fails closed."""

import hashlib
import json
from pathlib import Path

MODEL_CONFIG = dict(vocab=260, latent=16, width=64, heads=4, layers=2, budget=256)


def canonical_hash(value):
    return hashlib.sha256(
        json.dumps(value, sort_keys=True, separators=(",", ":")).encode()
    ).hexdigest()


def source_hash(names):
    root = Path(__file__).parent
    return canonical_hash(
        {name: hashlib.sha256((root / name).read_bytes()).hexdigest() for name in names}
    )


def training_identity(lr=0.001, seed=7, device="cpu"):
    import numpy as np
    import torch

    contract = dict(
        schema_version=2,
        model=MODEL_CONFIG,
        training_code_sha256=source_hash(["train.py", "model.py", "losses.py", "identity.py"]),
        objective=dict(
            text="autoregressive_ce",
            media="linear_flow_mse",
            alignment="symmetric_contrastive",
            alignment_weight=0.05,
        ),
        optimizer=dict(
            name="AdamW", lr=lr, betas=[0.9, 0.999], eps=1e-8, weight_decay=0.01, clip_norm=1.0
        ),
        seed=seed,
        precision="float32",
        torch=str(torch.__version__),
        numpy=str(np.__version__),
        device_family=device,
    )
    return dict(contract=contract, sha256=canonical_hash(contract))


def validate_resume(checkpoint, expected):
    if checkpoint.get("training_identity") != expected:
        raise ValueError(
            "resume computation identity mismatch (code/model/objective/optimizer/precision/runtime)"
        )
