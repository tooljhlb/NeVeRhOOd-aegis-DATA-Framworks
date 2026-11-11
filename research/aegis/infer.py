import argparse

import numpy as np
import torch

from .model import UnifiedModel


def main():
    p = argparse.ArgumentParser()
    p.add_argument("checkpoint")
    p.add_argument("prompt")
    p.add_argument("--modality", choices=["text", "video", "audio"], default="text")
    p.add_argument("--out", default="sample.npy")
    p.add_argument("--tokens", type=int, default=16)
    a = p.parse_args()
    if not 1 <= a.tokens <= 128:
        p.error("tokens must be 1..128")
    model = UnifiedModel().eval()
    model.load_state_dict(torch.load(a.checkpoint, map_location="cpu", weights_only=True)["model"])
    ids = torch.tensor([[b + 4 for b in a.prompt.encode()]], dtype=torch.long)
    if not 1 <= ids.shape[1] <= 128:
        p.error("prompt must be 1..128 UTF-8 bytes")
    if a.modality == "text":
        result = model.generate(ids, a.tokens)[0, ids.shape[1] :].tolist()
        print(bytes([i - 4 for i in result if i >= 4]).decode(errors="replace"))
    else:
        latent = model.sample(ids, (1, a.tokens, 16), {"video": 1, "audio": 2}[a.modality])
        np.save(a.out, latent.numpy())
        print("Saved codec latents to " + a.out + "; external matching decoder required.")


if __name__ == "__main__":
    main()
