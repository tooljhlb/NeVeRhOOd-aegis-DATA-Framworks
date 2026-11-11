"""Small real-source corpus pilot using native preparation and untouched validation.
This is a local engineering experiment, not an external coding benchmark.
"""

import argparse
import hashlib
import json
import os
import re
import subprocess
import sys
from pathlib import Path


def sha(data):
    return hashlib.sha256(data).hexdigest()


def main():
    p = argparse.ArgumentParser()
    p.add_argument("corpus", type=Path)
    p.add_argument("--out", type=Path, required=True)
    p.add_argument("--license-reference", required=True)
    p.add_argument("--steps", type=int, default=20)
    p.add_argument("--prepare-only", action="store_true")
    p.add_argument("--frozen-manifest", type=Path)
    a = p.parse_args()
    if not 1 <= a.steps <= 10000:
        p.error("steps must be 1..10000")
    repo = Path(__file__).resolve().parents[1]
    source = a.corpus.resolve()
    out = a.out.resolve()
    out.mkdir(parents=True, exist_ok=False)
    docs = []
    excluded = {"target", "build", "node_modules", ".venv", ".git", "__pycache__", "vendor"}
    for root, dirs, files in os.walk(source, followlinks=False):
        dirs[:] = sorted(d for d in dirs if d not in excluded and not (Path(root) / d).is_symlink())
        if Path(root).is_relative_to(out):
            dirs.clear()
            continue
        for name in sorted(files):
            f = Path(root) / name
            if f.suffix not in {".rs", ".py", ".cpp", ".hpp", ".c", ".js", ".ts", ".go", ".java"}:
                continue
            if f.is_symlink() or f.stat().st_size > 1024 * 1024:
                continue
            raw = f.read_bytes()
            text = raw.decode("utf-8")
            words = re.findall(r"\w+", text)
            shingles = {tuple(words[i : i + 5]) for i in range(max(0, len(words) - 4))}
            docs.append(
                dict(path=str(f.relative_to(source)), text=text, sha256=sha(raw), shingles=shingles)
            )
            if len(docs) > 500:
                raise ValueError(
                    "reference pilot limited to 500 documents; use sharded production loading"
                )
    if not docs:
        raise ValueError("no source documents")
    # Group exact and high-overlap documents before assigning a split.
    parent = list(range(len(docs)))

    def find(i):
        while parent[i] != i:
            parent[i] = parent[parent[i]]
            i = parent[i]
        return i

    near = []
    for i, x in enumerate(docs):
        for j in range(i):
            y = docs[j]
            union = x["shingles"] | y["shingles"]
            similarity = len(x["shingles"] & y["shingles"]) / len(union) if union else 0
            if x["sha256"] == y["sha256"] or similarity >= 0.8:
                parent[find(i)] = find(j)
                near.append(dict(a=x["path"], b=y["path"], jaccard=similarity))
    frozen = {}
    frozen_hash = None
    if a.frozen_manifest:
        frozen_hash = sha(a.frozen_manifest.read_bytes())
        frozen = {r["path"]: r for r in json.loads(a.frozen_manifest.read_text())["sources"]}
        if set(frozen) != {d["path"] for d in docs}:
            raise ValueError("frozen corpus membership changed")
        for d in docs:
            if d["sha256"] != frozen[d["path"]]["sha256"]:
                raise ValueError("frozen source hash changed")
        clusters = {}
        for i, d in enumerate(docs):
            clusters.setdefault(find(i), set()).add(frozen[d["path"]]["group_id"])
        if any(len(v) != 1 for v in clusters.values()):
            raise ValueError("frozen split separates duplicate groups")
    groups = {}
    for i, doc in enumerate(docs):
        groups.setdefault(find(i), []).append(doc["sha256"])
    entries = [{"path": d["path"], "sha256": d["sha256"]} for d in docs]
    revision = sha(json.dumps(entries, sort_keys=True).encode())
    manifest = dict(
        schema_version=1,
        revision=revision,
        license_reference=a.license_reference,
        sources=entries,
        near_duplicate_pairs=near,
        near_duplicate_threshold=0.8,
        frozen_manifest_sha256=frozen_hash,
        limits="Whole-document lexical shingles only; no semantic or external benchmark decontamination.",
    )
    (out / "sources.json").write_text(json.dumps(manifest, indent=2) + "\n")
    with (out / "source.jsonl").open("w") as f:
        for i, d in enumerate(docs):
            f.write(
                json.dumps(
                    dict(
                        text=d["text"],
                        source=d["path"],
                        license=a.license_reference,
                        revision=revision,
                        group_id=frozen[d["path"]]["group_id"] if frozen else min(groups[find(i)]),
                        content_kind="code",
                        normalization="preserve",
                    )
                )
                + "\n"
            )
    native = [
        "cargo",
        "run",
        "--locked",
        "--quiet",
        "--manifest-path",
        str(repo / "Cargo.toml"),
        "--",
    ]
    subprocess.run(
        native + ["prepare", str(out / "source.jsonl"), str(out / "prepared")], check=True
    )
    subprocess.run(native + ["validate-dataset", str(out / "prepared")], check=True)
    for split in ("train", "validation"):
        subprocess.run(
            native
            + [
                "tokenize",
                str(out / "prepared" / (split + ".pb")),
                str(out / (split + ".npy")),
                "--tokens",
                "16",
                "--max-rows",
                "256",
            ],
            check=True,
        )
    if a.prepare_only:
        print(out / "prepared/manifest.json")
        return
    env = dict(os.environ, PYTHONPATH=str(repo / "research"))

    def module(name, *args):
        subprocess.run([sys.executable, "-m", name, *map(str, args)], env=env, check=True)

    # Measure baseline before optimization; the validation split never enters train.
    module("aegis.heldout", out / "validation.npy", "--out", out / "baseline.json")
    module(
        "aegis.train",
        out / "train.npy",
        "--out",
        out / "continuous",
        "--steps",
        a.steps,
        "--batch",
        "2",
    )
    module(
        "aegis.heldout",
        out / "validation.npy",
        "--checkpoint",
        out / "continuous/checkpoint.pt",
        "--out",
        out / "candidate.json",
    )
    if a.steps > 1:
        first = a.steps // 2
        module(
            "aegis.train",
            out / "train.npy",
            "--out",
            out / "resumed",
            "--steps",
            first,
            "--batch",
            "2",
        )
        module(
            "aegis.train",
            out / "train.npy",
            "--out",
            out / "resumed",
            "--steps",
            a.steps,
            "--batch",
            "2",
            "--resume",
            out / "resumed/checkpoint.pt",
        )
        subprocess.run(
            [
                sys.executable,
                str(repo / "scripts/verify_resume.py"),
                str(out / "continuous/checkpoint.pt"),
                str(out / "resumed/checkpoint.pt"),
            ],
            check=True,
        )
    baseline = json.loads((out / "baseline.json").read_text())
    candidate = json.loads((out / "candidate.json").read_text())
    report = dict(
        schema_version=1,
        corpus_revision=revision,
        baseline=baseline,
        candidate=candidate,
        delta_nll=candidate["mean_nll"] - baseline["mean_nll"],
        exact_cpu_resume_verified=a.steps > 1,
        synthetic_data=False,
        limitation="Small same-repository source pilot; initialization vs trained checkpoint. Not a matched-budget architecture comparison or a general coding benchmark.",
    )
    train_receipt = json.loads((out / "continuous/receipt.json").read_text())
    for label, measured, updates, seconds in [
        ("baseline", baseline, 0, 0.0),
        ("candidate", candidate, a.steps, train_receipt["seconds"]),
    ]:
        experiment = dict(
            id=label,
            base_model_sha256=baseline["model_state_sha256"],
            model_sha256=measured["model_state_sha256"],
            training_data_sha256=sha((out / "train.npy").read_bytes()),
            evaluation_data_sha256=measured["dataset_sha256"],
            tokenizer_sha256=sha(b"utf8-byte-plus4-v1"),
            objective_sha256=sha(b"autoregressive_cross_entropy_v1"),
            schedule_sha256=sha(b"adamw_lr0.001_batch2_seed7_v1"),
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
        (out / ("experiment-" + label + ".json")).write_text(
            json.dumps(experiment, indent=2) + "\n"
        )
    subprocess.run(
        native
        + [
            "compare",
            str(out / "experiment-baseline.json"),
            str(out / "experiment-candidate.json"),
            "--out",
            str(out / "experiment-comparison.json"),
        ],
        check=True,
    )
    (out / "comparison.json").write_text(json.dumps(report, indent=2) + "\n")
    print(out / "comparison.json")


if __name__ == "__main__":
    main()
