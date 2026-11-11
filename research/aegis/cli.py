import argparse
import hashlib
import json
import os
import platform
import time
from pathlib import Path


def train(args):
    import torch

    from .identity import canonical_hash, source_hash, training_identity, validate_resume
    from .losses import cross_entropy, flow_loss
    from .model import UnifiedModel

    torch.set_num_threads(2)
    torch.manual_seed(7)
    device = torch.device(args.device)
    model = UnifiedModel().to(device)
    opt = torch.optim.AdamW(model.parameters(), lr=0.001)
    computation = training_identity(device=device.type)
    computation["contract"]["entrypoint"] = "synthetic_smoke"
    computation["contract"]["entrypoint_code_sha256"] = source_hash(["cli.py"])
    computation["sha256"] = canonical_hash(computation["contract"])
    start = 0
    if args.resume:
        ck = torch.load(args.resume, map_location="cpu", weights_only=True)
        validate_resume(ck, computation)
        model.load_state_dict(ck["model"])
        opt.load_state_dict(ck["optimizer"])
        start = ck["step"]
        torch.set_rng_state(ck["rng"])
        if device.type == "cuda":
            torch.cuda.set_rng_state_all(ck["cuda_rng"])
    if args.steps <= start:
        raise ValueError("steps must exceed saved step")
    # Explicit synthetic fixture: this command verifies mechanics, never model quality.
    losses = []
    begin = time.time()
    for step in range(start, args.steps):
        ids = torch.randint(4, 260, (4, 17), device=device)
        video = torch.randn(4, 8, 16, device=device)
        audio = torch.randn(4, 8, 16, device=device)
        loss = (
            cross_entropy(model.text_logits(ids[:, :-1]), ids[:, 1:])
            + flow_loss(model, ids[:, :-1], video, 1)
            + flow_loss(model, ids[:, :-1], audio, 2)
        )
        opt.zero_grad()
        loss.backward()
        torch.nn.utils.clip_grad_norm_(model.parameters(), 1.0, error_if_nonfinite=True)
        if not torch.isfinite(loss):
            raise RuntimeError("nonfinite loss")
        opt.step()
        losses.append(float(loss.detach()))
    out = Path(args.out)
    out.mkdir(parents=True, exist_ok=True)
    target = out / "checkpoint.pt"
    tmp = out / "checkpoint.tmp"
    torch.save(
        {
            "training_identity": computation,
            "model": model.state_dict(),
            "optimizer": opt.state_dict(),
            "step": args.steps,
            "rng": torch.get_rng_state(),
            "cuda_rng": torch.cuda.get_rng_state_all() if device.type == "cuda" else [],
        },
        tmp,
    )
    os.replace(tmp, target)
    receipt = {
        "kind": "synthetic_mechanics_only",
        "training_identity": computation,
        "step": args.steps,
        "executed_steps": len(losses),
        "losses": losses,
        "seconds": time.time() - begin,
        "torch": torch.__version__,
        "device": str(device),
        "platform": platform.platform(),
        "checkpoint_sha256": hashlib.sha256(target.read_bytes()).hexdigest(),
        "frontier_quality": "not_measured",
    }
    (out / "receipt.json").write_text(json.dumps(receipt, indent=2))
    print(json.dumps(receipt, indent=2))


def main():
    p = argparse.ArgumentParser()
    sub = p.add_subparsers(dest="cmd", required=True)
    t = sub.add_parser("smoke-train")
    t.add_argument("--steps", type=int, default=4)
    t.add_argument("--out", default="runs/smoke")
    t.add_argument("--resume")
    t.add_argument("--device", default="cpu")
    e = sub.add_parser("evaluate")
    e.add_argument("input")
    e.add_argument("--out", required=True)
    d = sub.add_parser("prepare")
    d.add_argument("input")
    d.add_argument("out")
    a = p.parse_args()
    if a.cmd == "smoke-train":
        if a.steps < 1:
            p.error("steps must be positive")
        train(a)
    elif a.cmd == "prepare":
        from .data import prepare

        print(prepare(a.input, a.out))
    else:
        from .evaluate import score

        source = Path(a.input)
        rows = [json.loads(s) for s in source.read_text().splitlines() if s.strip()]
        result = {
            "input_sha256": hashlib.sha256(source.read_bytes()).hexdigest(),
            "metrics": score(rows),
            "unrun": ["SWE-bench", "MM-Vet", "VBench", "waveform_FAD", "G-Eval"],
            "note": "RAG values are supplied judgments; tool/conversation metrics use labeled exact match.",
        }
        Path(a.out).write_text(json.dumps(result, indent=2))
        print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
