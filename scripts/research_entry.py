"""Explicit native-operator bridge to the isolated research modules."""

import runpy
import sys
from pathlib import Path

if len(sys.argv) < 2 or sys.argv[1] not in {"aegis.train", "aegis.heldout", "aegis.infer"}:
    raise SystemExit("expected aegis.train, aegis.heldout or aegis.infer")
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "research"))
module = sys.argv.pop(1)
runpy.run_module(module, run_name="__main__")
