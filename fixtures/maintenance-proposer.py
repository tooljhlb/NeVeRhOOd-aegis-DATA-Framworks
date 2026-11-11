"""Deterministic protocol fixture, not an AI model or general repair algorithm."""

import json
from pathlib import Path

request = json.loads(Path("aegis-request.json").read_text())
assert "sum.sh" in request["editable"]
Path("patch.json").write_text(
    json.dumps(
        {"edits": [{"path": "sum.sh", "before": "$1 - $2", "after": "$1 + $2", "occurrences": 1}]}
    )
)
