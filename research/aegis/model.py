"""Shared transformer with information-safe text and flow passes.
Inputs are codec latents, never raw videos. Codec training is external.
"""

import torch
from torch import nn


class UnifiedModel(nn.Module):
    def __init__(self, vocab=260, latent=16, width=64, heads=4, layers=2, budget=256):
        super().__init__()
        self.budget, self.latent = budget, latent
        self.text = nn.Embedding(vocab, width)
        self.modality = nn.Embedding(3, width)
        self.position = nn.Embedding(budget, width)
        self.time = nn.Sequential(nn.Linear(1, width), nn.SiLU(), nn.Linear(width, width))
        self.inputs = nn.ModuleList([nn.Linear(latent, width) for _ in range(2)])
        self.outputs = nn.ModuleList([nn.Linear(width, latent) for _ in range(2)])
        self.backbone = nn.TransformerEncoder(
            nn.TransformerEncoderLayer(
                width, heads, width * 4, dropout=0.0, batch_first=True, norm_first=True
            ),
            layers,
            enable_nested_tensor=False,
        )
        self.lm = nn.Linear(width, vocab)

    def encode(self, x, modality, causal=False, t=None):
        if x.shape[1] > self.budget:
            raise ValueError("token budget exceeded")
        x = (
            x
            + self.modality.weight[modality]
            + self.position(torch.arange(x.shape[1], device=x.device))
        )
        if t is not None:
            x = x + self.time(t.reshape(-1, 1)).unsqueeze(1)
        mask = (
            torch.triu(torch.ones(x.shape[1], x.shape[1], device=x.device, dtype=torch.bool), 1)
            if causal
            else None
        )
        return self.backbone(x, mask=mask)

    def text_logits(self, ids):
        return self.lm(self.encode(self.text(ids), 0, causal=True))

    def velocity(self, ids, noisy, t, modality):
        if modality not in (1, 2):
            raise ValueError("flow modality must be video=1 or audio=2")
        # Clean target text conditions flow; no media target is exposed to the text loss.
        a = self.text(ids) + self.modality.weight[0]
        b = self.inputs[modality - 1](noisy) + self.modality.weight[modality]
        x = torch.cat([a, b], 1)
        if x.shape[1] > self.budget:
            raise ValueError("joint token budget exceeded")
        x = (
            x
            + self.position(torch.arange(x.shape[1], device=x.device))
            + self.time(t[:, None])[:, None, :]
        )
        h = self.backbone(x)
        return self.outputs[modality - 1](h[:, ids.shape[1] :])

    @torch.no_grad()
    def generate(self, ids, count=16):
        for _ in range(count):
            ids = torch.cat(
                [
                    ids,
                    self.text_logits(ids[:, -self.budget :])[:, -1].argmax(-1, keepdim=True),
                ],
                1,
            )
        return ids

    @torch.no_grad()
    def sample(self, ids, shape, modality, steps=32):
        if steps < 1:
            raise ValueError("steps must be positive")
        x = torch.randn(shape, device=ids.device)
        for i in range(steps):
            t = torch.full((shape[0],), i / steps, device=x.device)
            x = x + self.velocity(ids, x, t, modality) / steps
        return x
