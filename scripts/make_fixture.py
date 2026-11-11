"""Reproducible synthetic trainer fixture. Not a real training corpus."""

import argparse

import numpy as np

p = argparse.ArgumentParser()
p.add_argument("out")
a = p.parse_args()
r = np.random.default_rng(7)
np.savez(
    a.out,
    text=r.integers(4, 260, (8, 17), dtype=np.int64),
    video=r.standard_normal((8, 8, 16)).astype("float32"),
    audio=r.standard_normal((8, 8, 16)).astype("float32"),
)
