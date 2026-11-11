"""Deterministic labeled-data metrics. No lexical proxy is called faithfulness."""

import math


def score(rows):
    if not rows:
        raise ValueError("empty evaluation")
    sums = {}
    counts = {}

    def add(k, v):
        if not math.isfinite(float(v)):
            raise ValueError("nonfinite metric")
        sums[k] = sums.get(k, 0) + float(v)
        counts[k] = counts.get(k, 0) + 1

    groups = {}
    for r in rows:
        task = r["task"]
        y = r["target"]
        p = r["prediction"]
        if task in ("binary", "multiclass", "exam", "conversation", "tool"):
            correct = p == y
            add(task + "/accuracy", correct)
            if task == "binary" and "group" in r:
                groups.setdefault(r["group"], []).append(int(correct))
        elif task == "regression":
            add("regression/mse", (float(p) - float(y)) ** 2)
        elif task == "text":
            import sacrebleu
            from rouge_score.rouge_scorer import RougeScorer

            add("text/sentence_bleu", sacrebleu.sentence_bleu(p, [y]).score)
            add(
                "text/rougeL",
                RougeScorer(["rougeL"], use_stemmer=True).score(y, p)["rougeL"].fmeasure,
            )
        elif task == "rag":
            # Human annotations or independently reviewed entailment judgments required.
            for name in ("faithfulness", "answer_relevance", "context_relevance"):
                value = r["judgments"][name]
                if not 0 <= value <= 1:
                    raise ValueError("judgment outside [0,1]")
                add("rag/" + name, value)
            if not r.get("judge_id"):
                raise ValueError("judge provenance missing")
        else:
            raise ValueError("unsupported task: " + task)
    result = {k: {"value": sums[k] / counts[k], "n": counts[k]} for k in sums}
    if len(groups) > 1:
        rates = [sum(v) / len(v) for v in groups.values()]
        result["bias/group_accuracy_gap"] = {
            "value": max(rates) - min(rates),
            "groups": {k: len(v) for k, v in groups.items()},
        }
    return result


def frechet(real, generated):
    import numpy as np

    # Stable PSD sandwich rather than taking a square root of a nonsymmetric product.
    a, b = np.asarray(real, dtype=float), np.asarray(generated, dtype=float)
    if a.ndim != 2 or b.ndim != 2 or a.shape[1] != b.shape[1] or min(len(a), len(b)) < 2:
        raise ValueError("need >=2 matching embedding vectors")
    if not np.isfinite(a).all() or not np.isfinite(b).all():
        raise ValueError("nonfinite embeddings")
    ca, cb = (
        np.atleast_2d(np.cov(a, rowvar=False)),
        np.atleast_2d(np.cov(b, rowvar=False)),
    )
    vals, vecs = np.linalg.eigh(ca)
    root = (vecs * np.sqrt(vals.clip(0))) @ vecs.T
    sandwich = root @ cb @ root
    trace_root = np.sqrt(np.linalg.eigvalsh((sandwich + sandwich.T) / 2).clip(0)).sum()
    return float(
        max(
            0,
            ((a.mean(0) - b.mean(0)) ** 2).sum() + np.trace(ca) + np.trace(cb) - 2 * trace_root,
        )
    )
