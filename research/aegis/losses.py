import torch
import torch.nn.functional as F


def cross_entropy(logits, targets):
    return F.cross_entropy(logits.reshape(-1, logits.shape[-1]), targets.reshape(-1))


def flow_loss(model, ids, clean, modality):
    noise = torch.randn_like(clean)
    t = torch.rand(clean.shape[0], device=clean.device)
    z = (1 - t[:, None, None]) * noise + t[:, None, None] * clean
    return F.mse_loss(model.velocity(ids, z, t, modality), clean - noise)


def dpo(chosen, rejected, ref_chosen, ref_rejected, beta=0.1):
    # Inputs are summed response-only log probabilities, not token logits.
    return -F.logsigmoid(beta * ((chosen - rejected) - (ref_chosen - ref_rejected))).mean()


def ppo_loss(logp, old_logp, advantages, clip=0.2):
    ratio = (logp - old_logp.detach()).exp()
    return -torch.minimum(
        ratio * advantages.detach(),
        ratio.clamp(1 - clip, 1 + clip) * advantages.detach(),
    ).mean()


def contrastive(a, b, temperature=0.07):
    if a.shape[0] < 2:
        raise ValueError("contrastive loss needs negative pairs")
    logits = F.normalize(a, dim=-1) @ F.normalize(b, dim=-1).T / temperature
    labels = torch.arange(a.shape[0], device=a.device)
    return (F.cross_entropy(logits, labels) + F.cross_entropy(logits.T, labels)) / 2


def spectral(pred, target, sizes=(64, 128, 256)):
    if pred.shape != target.shape:
        raise ValueError("waveform shapes differ")
    result = pred.new_zeros(())
    for size in sizes:
        if pred.shape[-1] < size:
            raise ValueError("waveform shorter than FFT window")
        window = torch.hann_window(size, device=pred.device)
        a = torch.stft(pred, size, hop_length=size // 4, window=window, return_complex=True).abs()
        b = torch.stft(target, size, hop_length=size // 4, window=window, return_complex=True).abs()
        result = (
            result + F.mse_loss(a, b) + F.l1_loss(a.clamp_min(1e-6).log(), b.clamp_min(1e-6).log())
        )
    return result / len(sizes)
