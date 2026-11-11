"""Train aligned NPZ batches with exact CPU resume and immutable data identity."""

import argparse
import hashlib
import json
import os
import sys
import time
from pathlib import Path

import numpy as np
import torch

from .identity import MODEL_CONFIG, training_identity, validate_resume
from .losses import contrastive, cross_entropy, flow_loss
from .model import UnifiedModel

try:
    import resource
except ImportError:
    resource = None


def digest(path):
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for block in iter(lambda: f.read(1024 * 1024), b""):
            h.update(block)
    return h.hexdigest()


def main():
    p = argparse.ArgumentParser()
    p.add_argument("data")
    p.add_argument("--out", required=True)
    p.add_argument("--steps", type=int, default=100)
    p.add_argument("--batch", type=int, default=4)
    p.add_argument("--device", default="cpu")
    p.add_argument("--resume")
    p.add_argument("--save-every", type=int, default=10)
    p.add_argument("--lr", type=float, default=0.001)
    p.add_argument("--seed", type=int, default=7)
    a = p.parse_args()
    if min(a.steps, a.batch, a.save_every) < 1:
        p.error("positive counts required")
    torch.set_num_threads(2)
    if not np.isfinite(a.lr) or a.lr <= 0:
        p.error("positive finite learning rate required")
    torch.manual_seed(a.seed)
    device = torch.device(a.device)
    loaded = np.load(a.data, allow_pickle=False)
    data_manifest_sha256 = None
    if isinstance(loaded, np.ndarray):
        sidecar = Path(str(a.data) + ".manifest.json")
        manifest = json.loads(sidecar.read_text())
        if (
            manifest.get("output_sha256") != digest(a.data)
            or manifest.get("tokenizer") != "utf8-byte-plus4-v1"
            or tuple(loaded.shape) != (manifest.get("rows"), manifest.get("tokens_per_row"))
        ):
            raise ValueError("native export manifest mismatch")
        data_manifest_sha256 = digest(sidecar)
        data = {"text": torch.from_numpy(loaded.copy())}
    else:
        with loaded as f:
            data = {k: torch.from_numpy(f[k].copy()) for k in f.files}
    ids = data["text"]
    if (
        ids.ndim != 2
        or ids.shape[1] < 2
        or ids.dtype != torch.int64
        or ids.min() < 0
        or ids.max() >= 260
    ):
        raise ValueError("text must be int64 [N,T>=2], IDs 0..259")
    if len(ids) < a.batch:
        raise ValueError("not enough examples")
    for k in ("video", "audio"):
        if k in data and (
            data[k].ndim != 3
            or data[k].shape[0] != len(ids)
            or data[k].shape[-1] != 16
            or not data[k].is_floating_point()
            or not torch.isfinite(data[k]).all()
        ):
            raise ValueError("invalid " + k + " latents")
    computation = training_identity(a.lr, a.seed, device.type)
    model = UnifiedModel(**MODEL_CONFIG).to(device)
    opt = torch.optim.AdamW(model.parameters(), lr=a.lr)
    identity = digest(a.data)
    start = 0
    if a.resume:
        ck = torch.load(a.resume, map_location="cpu", weights_only=True)
        validate_resume(ck, computation)
        if (
            ck["data_sha256"] != identity
            or ck["batch"] != a.batch
            or ck.get("data_manifest_sha256") != data_manifest_sha256
        ):
            raise ValueError("resume data/batch mismatch")
        model.load_state_dict(ck["model"])
        opt.load_state_dict(ck["optimizer"])
        start = ck["step"]
        torch.set_rng_state(ck["rng"])
        if device.type == "cuda":
            torch.cuda.set_rng_state_all(ck["cuda_rng"])
    if a.steps <= start:
        raise ValueError("steps must exceed saved step")
    out = Path(a.out)
    out.mkdir(parents=True, exist_ok=True)
    if (out / "checkpoint.pt").exists() and not a.resume:
        raise ValueError("checkpoint already exists; resume explicitly or choose a new output")
    if a.resume and (out / "metrics.jsonl").exists():
        valid = []
        for line in (out / "metrics.jsonl").read_text().splitlines():
            try:
                row = json.loads(line)
            except json.JSONDecodeError:
                continue
            if row.get("step", start + 1) <= start:
                valid.append(json.dumps(row))
        (out / "metrics.jsonl").write_text("\n".join(valid) + ("\n" if valid else ""))
    if device.type == "cuda":
        torch.cuda.reset_peak_memory_stats(device)
        torch.cuda.synchronize(device)
    begin = time.time()
    log = (out / "metrics.jsonl").open("a")
    try:
        for step in range(start, a.steps):
            index = torch.randint(len(ids), (a.batch,))
            text = ids[index].to(device)
            losses = {"text": cross_entropy(model.text_logits(text[:, :-1]), text[:, 1:])}
            for modality, k in ((1, "video"), (2, "audio")):
                if k in data:
                    losses[k] = flow_loss(model, text[:, :-1], data[k][index].to(device), modality)
            # Shared-space alignment uses the same backbone and modality input projections.
            if a.batch > 1:
                text_emb = model.encode(model.text(text[:, :-1]), 0, causal=True).mean(1)
                for modality, k in ((1, "video"), (2, "audio")):
                    if k in data:
                        media = model.inputs[modality - 1](data[k][index].to(device))
                        media_emb = model.encode(media, modality).mean(1)
                        losses[k + "_alignment"] = 0.05 * contrastive(text_emb, media_emb)
            loss = sum(losses.values())
            opt.zero_grad()
            loss.backward()
            torch.nn.utils.clip_grad_norm_(model.parameters(), 1.0, error_if_nonfinite=True)
            if not torch.isfinite(loss):
                raise RuntimeError("nonfinite loss")
            opt.step()
            log.write(
                json.dumps(
                    {
                        "step": step + 1,
                        **{k: float(v.detach()) for k, v in losses.items()},
                    }
                )
                + "\n"
            )
            log.flush()
            if (step + 1) % a.save_every == 0 or step + 1 == a.steps:
                tmp = out / "checkpoint.tmp"
                target = out / "checkpoint.pt"
                torch.save(
                    {
                        "training_identity": computation,
                        "model": model.state_dict(),
                        "optimizer": opt.state_dict(),
                        "step": step + 1,
                        "rng": torch.get_rng_state(),
                        "cuda_rng": torch.cuda.get_rng_state_all() if device.type == "cuda" else [],
                        "data_sha256": identity,
                        "data_manifest_sha256": data_manifest_sha256,
                        "batch": a.batch,
                    },
                    tmp,
                )
                with tmp.open("rb") as checkpoint_file:
                    os.fsync(checkpoint_file.fileno())
                os.replace(tmp, target)
                if os.name == "posix":
                    descriptor = os.open(out, os.O_RDONLY)
                    try:
                        os.fsync(descriptor)
                    finally:
                        os.close(descriptor)
        if device.type == "cuda":
            torch.cuda.synchronize(device)
        elapsed = time.time() - begin
        receipt = {
            "executed_steps": a.steps - start,
            "steps_per_second": (a.steps - start) / elapsed,
            "text_target_tokens_per_second": ((a.steps - start) * a.batch * (ids.shape[1] - 1))
            / elapsed,
            "timing_scope": "optimizer loop and checkpoint I/O; excludes initialization/loading",
            "peak_host_rss_bytes": resource.getrusage(resource.RUSAGE_SELF).ru_maxrss * 1024
            if sys.platform == "linux"
            else None,
            "peak_gpu_allocated_bytes": torch.cuda.max_memory_allocated(device)
            if device.type == "cuda"
            else None,
            "training_identity": computation,
            "data_sha256": identity,
            "data_manifest_sha256": data_manifest_sha256,
            "checkpoint_sha256": digest(target),
            "step": a.steps,
            "device": str(device),
            "seconds": elapsed,
            "torch": str(torch.__version__),
            "held_out_quality": "not_measured",
            "modalities": list(data),
        }
        (out / "receipt.json").write_text(json.dumps(receipt, indent=2))
        print(json.dumps(receipt, indent=2))
    finally:
        log.close()


if __name__ == "__main__":
    main()
