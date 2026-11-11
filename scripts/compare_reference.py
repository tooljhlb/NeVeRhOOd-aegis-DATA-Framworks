"""Create provenance-bound experiment records, comparisons, and acceptance inputs."""

import argparse
import hashlib
import json
from pathlib import Path


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def main():
    p = argparse.ArgumentParser()
    p.add_argument("out", type=Path)
    p.add_argument("--steps", type=int, required=True)
    a = p.parse_args()
    root = a.out
    baseline = json.loads((root / "baseline.json").read_text())
    candidate = json.loads((root / "candidate.json").read_text())
    recovery = json.loads((root / "resume-verification.json").read_text())
    receipt = json.loads((root / "continuous/receipt.json").read_text())
    if recovery["verified"] != 1 or recovery["continuous_sha256"] != candidate["checkpoint_sha256"]:
        raise ValueError("recovery proof does not match evaluated checkpoint")
    for label, measured, updates, seconds in [
        ("baseline", baseline, 0, 0.0),
        ("candidate", candidate, a.steps, receipt["seconds"]),
    ]:
        experiment = dict(
            id=label,
            base_model_sha256=baseline["model_state_sha256"],
            model_sha256=measured["model_state_sha256"],
            training_data_sha256=digest(root / "train.npy"),
            evaluation_data_sha256=measured["dataset_sha256"],
            tokenizer_sha256=hashlib.sha256(b"utf8-byte-plus4-v1").hexdigest(),
            objective_sha256=hashlib.sha256(b"autoregressive_cross_entropy_v1").hexdigest(),
            schedule_sha256=receipt["training_identity"]["sha256"],
            evaluator_code_sha256=measured["evaluator_code_sha256"],
            evaluator_config_sha256=measured["evaluator_config_sha256"],
            decoding_sha256=measured["decoding_sha256"],
            seed=7,
            updates=updates,
            wall_seconds=seconds,
            metrics={
                "code/heldout_nll": dict(
                    value=measured["mean_nll"], n=measured["n_target_tokens"], direction="max"
                )
            },
        )
        (root / f"experiment-{label}.json").write_text(json.dumps(experiment, indent=2) + "\n")
    result = dict(
        schema_version=2,
        baseline=baseline,
        candidate=candidate,
        resume_verified=1,
        delta_nll=candidate["mean_nll"] - baseline["mean_nll"],
        scope="Small source-language-loss engineering experiment; not coding-task capability or matched-budget architecture superiority.",
    )
    result_path = root / "comparison.json"
    result_path.write_text(json.dumps(result, indent=2) + "\n")
    gates = [
        dict(
            name="heldout_nonregression",
            direction="max",
            threshold=0.0,
            metric_pointer="/delta_nll",
            evidence_file=str(result_path),
            evidence_sha256=digest(result_path),
        ),
        dict(
            name="exact_cpu_resume",
            direction="min",
            threshold=1.0,
            metric_pointer="/resume_verified",
            evidence_file=str(result_path),
            evidence_sha256=digest(result_path),
        ),
    ]
    (root / "gates.json").write_text(json.dumps(gates, indent=2) + "\n")


if __name__ == "__main__":
    main()
