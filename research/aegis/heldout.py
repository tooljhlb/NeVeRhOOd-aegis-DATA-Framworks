"""Fixed-checkpoint byte-language loss; no optimization or model selection."""

import argparse
import hashlib
import json
import math
from pathlib import Path

import numpy as np
import torch
import torch.nn.functional as F

from .identity import MODEL_CONFIG, canonical_hash, source_hash
from .model import UnifiedModel


def evaluate(data, checkpoint=None, seed=7):
    torch.set_num_threads(2)
    torch.manual_seed(seed)
    model = UnifiedModel(**MODEL_CONFIG).eval()
    if checkpoint:
        model.load_state_dict(
            torch.load(checkpoint, map_location="cpu", weights_only=True)["model"]
        )
    state_hash = hashlib.sha256()
    for name, tensor in sorted(model.state_dict().items()):
        state_hash.update(name.encode())
        state_hash.update(str(tensor.dtype).encode())
        state_hash.update(str(tuple(tensor.shape)).encode())
        state_hash.update(tensor.detach().cpu().contiguous().numpy().tobytes())
    raw = Path(data).read_bytes()
    manifest = json.loads(Path(str(data) + ".manifest.json").read_text())
    identity = hashlib.sha256(raw).hexdigest()
    if manifest["output_sha256"] != identity or manifest["tokenizer"] != "utf8-byte-plus4-v1":
        raise ValueError("heldout manifest mismatch")
    ids = np.load(data, allow_pickle=False)
    if ids.dtype != np.int64 or ids.ndim != 2 or len(ids) == 0 or ids.shape[1] < 2:
        raise ValueError("invalid heldout shape/dtype")
    if ids.min() < 0 or ids.max() >= 260:
        raise ValueError("invalid token IDs")
    losses = []
    with torch.inference_mode():
        for start in range(0, len(ids), 16):
            x = torch.from_numpy(ids[start : start + 16].copy())
            logits = model.text_logits(x[:, :-1])
            loss = F.cross_entropy(logits.reshape(-1, 260), x[:, 1:].reshape(-1), reduction="none")
            losses.extend(loss.reshape(len(x), -1).mean(1).tolist())
    values = np.asarray(losses, dtype=float)
    if not np.isfinite(values).all():
        raise ValueError("nonfinite heldout loss")
    groups = manifest.get("row_groups")
    if not groups or len(groups) != len(values):
        raise ValueError("document/duplicate-group row identities required")
    aggregate = {}
    for group, value in zip(groups, values):
        aggregate.setdefault(group, []).append(value)
    totals = np.array([sum(v) for v in aggregate.values()])
    counts = np.array([len(v) for v in aggregate.values()])
    rng = np.random.default_rng(seed)
    intervals = None
    if len(aggregate) > 1:
        boot = []
        for _ in range(500):
            selected = rng.integers(0, len(aggregate), len(aggregate))
            boot.append(totals[selected].sum() / counts[selected].sum())
        intervals = np.quantile(boot, [0.025, 0.975]).tolist()
    eval_config = dict(
        schema_version=2,
        batch=16,
        objective="next_byte_ce",
        uncertainty="duplicate_group_bootstrap",
        replicates=500,
        seed=seed,
        torch=str(torch.__version__),
        numpy=str(np.__version__),
    )
    return {
        "schema_version": 2,
        "evaluator_code_sha256": source_hash(["heldout.py", "model.py", "identity.py"]),
        "evaluator_config_sha256": canonical_hash(eval_config),
        "decoding_sha256": canonical_hash(
            {"mode": "teacher_forced", "generation_sampling": "none"}
        ),
        "model_state_sha256": state_hash.hexdigest(),
        "model_code_sha256": hashlib.sha256(
            Path(__file__).with_name("model.py").read_bytes()
        ).hexdigest(),
        "torch_version": str(torch.__version__),
        "dataset_sha256": identity,
        "checkpoint_sha256": hashlib.sha256(Path(checkpoint).read_bytes()).hexdigest()
        if checkpoint
        else None,
        "initialization_seed": seed,
        "n_windows": len(values),
        "n_target_tokens": int(len(ids) * (ids.shape[1] - 1)),
        "mean_nll": float(values.mean()),
        "perplexity": math.exp(float(values.mean())),
        "group_bootstrap_ci95": intervals,
        "n_duplicate_groups": len(aggregate),
        "uncertainty_limit": "Whole duplicate-group bootstrap; interval unavailable for fewer than two groups. This is not a capability benchmark.",
        "scope": "Heldout byte-language loss only; no executable coding/reasoning capability claim.",
    }


def main():
    p = argparse.ArgumentParser()
    p.add_argument("data")
    p.add_argument("--checkpoint")
    p.add_argument("--out", required=True)
    a = p.parse_args()
    result = evaluate(a.data, a.checkpoint)
    Path(a.out).write_text(json.dumps(result, indent=2) + "\n")


if __name__ == "__main__":
    main()
