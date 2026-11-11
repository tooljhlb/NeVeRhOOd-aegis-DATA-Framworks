import json
import tempfile
import unittest
from pathlib import Path

from aegis.data import prepare
from aegis.evaluate import frechet, score


class Metrics(unittest.TestCase):
    def test_scores(self):
        r = score(
            [
                dict(task="binary", target=1, prediction=1),
                dict(task="regression", target=2, prediction=4),
            ]
        )
        self.assertEqual(r["binary/accuracy"]["value"], 1)
        self.assertEqual(r["regression/mse"]["value"], 4)

    def test_missing_judge(self):
        with self.assertRaises((KeyError, ValueError)):
            score([dict(task="rag", target="", prediction="", judgments={})])

    def test_empty(self):
        with self.assertRaises(ValueError):
            score([])

    def test_dedup(self):
        with tempfile.TemporaryDirectory() as d:
            src = Path(d) / "in"
            row = dict(text="hello world", source="fixture", license="CC0")
            src.write_text((json.dumps(row) + "\n") * 2)
            r = prepare(src, Path(d) / "out")
            self.assertEqual(r["duplicate"], 1)

    def test_frechet_identity(self):
        self.assertAlmostEqual(
            frechet([[0, 1], [1, 0], [2, 2]], [[0, 1], [1, 0], [2, 2]]), 0, places=6
        )


class TorchChecks(unittest.TestCase):
    def test_causal_no_leak(self):
        import torch
        from aegis.model import UnifiedModel

        torch.manual_seed(1)
        m = UnifiedModel().eval()
        a = torch.tensor([[4, 5, 6, 7]])
        b = torch.tensor([[4, 5, 99, 100]])
        self.assertTrue(torch.allclose(m.text_logits(a)[:, :2], m.text_logits(b)[:, :2], atol=1e-6))

    def test_losses_backward(self):
        import torch
        from aegis.losses import contrastive, dpo, ppo_loss, spectral

        a = torch.randn(3, 8, requires_grad=True)
        loss = (
            dpo(a[:, 0], a[:, 1], a[:, 2], a[:, 3])
            + ppo_loss(a[:, 0], a[:, 1], a[:, 2])
            + contrastive(a, a)
        )
        loss.backward()
        self.assertTrue(torch.isfinite(a.grad).all())
        x = torch.randn(2, 512, requires_grad=True)
        z = spectral(x, torch.zeros_like(x))
        z.backward()
        self.assertTrue(torch.isfinite(x.grad).all())

    def test_budget(self):
        import torch
        from aegis.model import UnifiedModel

        with self.assertRaises(ValueError):
            UnifiedModel(budget=4).text_logits(torch.ones(1, 5, dtype=torch.long))


if __name__ == "__main__":
    unittest.main()
