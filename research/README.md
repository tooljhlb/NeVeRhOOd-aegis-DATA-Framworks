# AEGIS Multimodal Lab

The isolated Python/PyTorch research reference in this repository is an executable core for shared text/video/audio representation learning and honest model evaluation. It does not modify the active AEGIS repository, workers, credentials, or checkpoints. Python/PyTorch is used for architecture validation; this is not the native Rust/C++ production runtime.

## Run

Run these commands from `research/` after generating a fixture from the repository root with `python scripts/make_fixture.py research/examples/fixture.npz`.

```bash
python -m venv .venv
source .venv/bin/activate
pip install -e '.[eval,data]'
python -m unittest discover -s tests -v
python -m aegis.cli smoke-train --steps 4 --out runs/smoke-new
python -m aegis.train examples/fixture.npz --out runs/train-new --steps 100
python -m aegis.train examples/fixture.npz --out runs/train-new --steps 200 --resume runs/train-new/checkpoint.pt
python -m aegis.infer runs/train-new/checkpoint.pt hello
python -m aegis.infer runs/train-new/checkpoint.pt hello --modality video --out video-latents.npy
python -m aegis.cli evaluate examples/eval.jsonl --out evaluation.json
```

The bundled fixture is seeded synthetic data, not a training corpus. Its scores verify software plumbing only. The tiny model is untrained for meaningful language or media generation. Inference emits text bytes or latent arrays; playable media requires a matching trained codec and trained weights.

## Architecture and implemented behavior

One shared transformer, learned position/modality embeddings, causal text attention, time-conditioned bidirectional media flow passes, separate output heads, and contrastive text/media alignment. The text pass cannot see future tokens or clean target media. Training optimizes next-byte cross entropy and available media flow losses plus weighted alignment. Euler integration samples continuous media latents. Shared weights do not force all modalities into an identical decoding algorithm.

Inputs: NPZ `text` int64 [N,T] IDs 0..259 (UTF-8 byte + 4; 0..3 reserved), optional `video` / `audio` float32 [N,L,16]. Rows must be aligned. Text examples must be fixed length with no padding in this baseline. Context budget is 256 combined tokens. This implementation deliberately does not claim 1.14M context, distributed training, sparse routing, or production throughput.

`aegis.losses` also provides response-log-probability DPO, PPO clipped policy surrogate, symmetric contrastive loss, and multi-resolution waveform spectral loss. DPO/PPO/spectral losses are library functions, not secretly active in the default joint trainer. PPO still needs rollout collection, reward/value models, advantage estimation, and KL control. Spectral loss needs a differentiable waveform decoder. Flow matching and SDE score matching are different objectives; squared velocity error does not by itself guarantee faster or better generation.

## Data preparation

`python -m aegis.cli prepare input.jsonl new-output-dir` streams text records, normalizes whitespace, deduplicates using SQLite, and deterministically routes document groups to train/validation/test. Each row requires `text`, `source`, and `license`; optional `group_id` keeps related records together. This is exact deduplication, not fuzzy/semantic dedup or benchmark decontamination. Failed runs can leave partial output; use a fresh directory after fixing input.

`aegis.data.stream_hf(repo, revision, config, split, limit, destination, license_name)` provides bounded Hugging Face text ingestion with explicit revision and provenance. It does not scrape video links or grant media rights. See `docs/catalog.md` for media and benchmark integration boundaries.

## Scoring

`evaluate` handles labeled binary/multiclass/exam/tool/conversation exact match, regression MSE, sentence BLEU, ROUGE-L, and separately supplied RAG judgments with judge identity. Tool exact match checks a proposed call, not successful tool execution. Group accuracy gaps are descriptive diagnostics, not a universal fairness score. RAG judgments are aggregated, not independently generated. BLEU/ROUGE measure overlap, not factual truth.

`frechet(real_embeddings, generated_embeddings)` implements stable distributional FrÃ©chet distance. To call a result FAD, use a fixed audio embedding model, identical preprocessing, documented sample rates and reference set. This function does not extract audio embeddings.

`python -m aegis.benchmark --help` imports external official engine JSON outputs with engine revision, dataset hash, model hash, and result hash. Imported results remain `imported_unverified`. There is no fake MM-Vet, VBench, SWE-bench or G-Eval score.

## Public reference verification

This package includes CPU unit checks, synthetic training/recovery fixtures and small held-out examples. Run the public CI workflow at your chosen commit for independently reproducible evidence. Historical internal test receipts are not redistributed in this public edition. These reference checks do not establish external benchmark quality or frontier model capability.

## Remaining to reach the requested full system

1. Select and load real pretrained audio/video codecs with known latent shapes and scaling; adapt the model's 16-dimensional interfaces and verify encode/decode reconstruction.
2. Curate authorized text/video/music data, align timestamps, cache codec latents, build scalable sharded loaders, and decontaminate against held-out benchmarks.
3. Train on real data with validation, ablations, distributed execution, durable remote checkpoints, and measured GPU memory/throughput. The current trainer loads NPZ arrays into CPU RAM and is intended for small controlled experiments.
4. Install and execute official SWE-bench, MM-Vet, VBench, and audio embedding/FAD pipelines on actual generated artifacts. Build curated licensed exam suites and model inference adapters. No official SAT/GRE/GMAT/civil-service test content is bundled.
5. Add a calibrated local judge endpoint for relevance/faithfulness/G-Eval, human audits, bias slices, confidence intervals, and adversarial hallucination tests.
6. Add authenticated serving, cancellation/queues, media assembly, native runtime export, and production observability after the model paths are validated.

This reference is now included in the dedicated aegis-DATA-framworks repository. No remote GPU run or dataset download is automatically launched. This is a runnable first implementation, not a completed frontier model or a fully integrated production system.
