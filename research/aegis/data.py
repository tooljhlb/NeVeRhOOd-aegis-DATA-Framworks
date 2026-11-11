"""Streaming text ingestion, bounded exact dedup, stable split routing."""

import hashlib
import json
import sqlite3
from pathlib import Path


def prepare(source, destination):
    out = Path(destination)
    out.mkdir(parents=True, exist_ok=False)
    db = sqlite3.connect(out / "dedup.sqlite")
    db.execute("CREATE TABLE seen (sha TEXT PRIMARY KEY)")
    counts = {"train": 0, "validation": 0, "test": 0, "duplicate": 0}
    files = {k: (out / (k + ".jsonl")).open("w") for k in ("train", "validation", "test")}
    try:
        with open(source) as f:
            for line in f:
                row = json.loads(line)
                if not row.get("source") or not row.get("license"):
                    raise ValueError("source and license required")
                kind = row.get("content_kind", "code")
                normalization = row.get("normalization", "preserve")
                if (
                    kind not in {"code", "prose"}
                    or normalization not in {"preserve", "collapse_whitespace"}
                    or (kind == "code" and normalization != "preserve")
                ):
                    raise ValueError("code must be preserved; prose normalization must be explicit")
                original_sha256 = hashlib.sha256(row["text"].encode()).hexdigest()
                text = row["text"] if normalization == "preserve" else " ".join(row["text"].split())
                row = dict(
                    row,
                    content_kind=kind,
                    normalization=normalization,
                    original_sha256=original_sha256,
                )
                if not text:
                    continue
                sha = hashlib.sha256(text.encode()).hexdigest()
                try:
                    db.execute("INSERT INTO seen VALUES (?)", (sha,))
                except sqlite3.IntegrityError:
                    counts["duplicate"] += 1
                    continue
                # Same source document group always maps to one split.
                group = str(row.get("group_id", sha))
                bucket = int(hashlib.sha256(group.encode()).hexdigest()[:8], 16) % 100
                split = "test" if bucket < 5 else "validation" if bucket < 10 else "train"
                files[split].write(json.dumps(dict(row, text=text, sha256=sha)) + "\n")
                counts[split] += 1
        db.commit()
    finally:
        for f in files.values():
            f.close()
        db.close()
    (out / "manifest.json").write_text(json.dumps(counts, indent=2))
    return counts


def stream_hf(repo, revision, config, split, limit, destination, license_name):
    from datasets import load_dataset

    if not revision or not license_name or limit < 1:
        raise ValueError("revision, license and positive limit required")
    rows = load_dataset(repo, name=config, revision=revision, split=split, streaming=True)
    with open(destination, "x") as f:
        for i, row in enumerate(rows):
            if i >= limit:
                break
            if "text" not in row:
                raise ValueError(
                    "adapter requires text; media metadata needs explicit normalization"
                )
            f.write(
                json.dumps(
                    {
                        "text": row["text"],
                        "source": repo,
                        "license": license_name,
                        "revision": revision,
                        "group_id": row.get("id", i),
                    }
                )
                + "\n"
            )
