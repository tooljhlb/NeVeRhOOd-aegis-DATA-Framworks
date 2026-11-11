import copy
import hashlib
import json
import tempfile
import unittest
from pathlib import Path

import numpy as np
from aegis.heldout import evaluate
from aegis.identity import training_identity, validate_resume


class ComputationIdentity(unittest.TestCase):
    def test_changed_optimizer_or_code_rejected(self):
        identity = training_identity()
        validate_resume({"training_identity": identity}, identity)
        changed = copy.deepcopy(identity)
        changed["contract"]["optimizer"]["lr"] = 0.1
        with self.assertRaisesRegex(ValueError, "identity mismatch"):
            validate_resume({"training_identity": changed}, identity)
        changed = copy.deepcopy(identity)
        changed["contract"]["training_code_sha256"] = "0" * 64
        with self.assertRaises(ValueError):
            validate_resume({"training_identity": changed}, identity)
        with self.assertRaises(ValueError):
            validate_resume({}, identity)

    def test_group_uncertainty_requires_multiple_groups(self):
        with tempfile.TemporaryDirectory() as d:
            path = Path(d) / "heldout.npy"
            np.save(path, np.array([[4, 5, 6, 7], [8, 9, 10, 11]], dtype=np.int64))
            manifest = dict(
                output_sha256=hashlib.sha256(path.read_bytes()).hexdigest(),
                tokenizer="utf8-byte-plus4-v1",
                row_groups=["same", "same"],
            )
            sidecar = Path(str(path) + ".manifest.json")
            sidecar.write_text(json.dumps(manifest))
            result = evaluate(path)
            self.assertIsNone(result["group_bootstrap_ci95"])
            self.assertEqual(result["n_duplicate_groups"], 1)
            manifest["row_groups"] = ["a", "b"]
            sidecar.write_text(json.dumps(manifest))
            result = evaluate(path)
            self.assertEqual(result["n_duplicate_groups"], 2)
            self.assertEqual(len(result["group_bootstrap_ci95"]), 2)


if __name__ == "__main__":
    unittest.main()
