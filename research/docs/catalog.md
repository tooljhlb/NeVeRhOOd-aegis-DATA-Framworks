# Dataset and evaluation integration directory

Pin an immutable upstream revision and record source terms before ingestion. Dataset annotation licenses do not necessarily license linked media. Store provenance, content hash, document/clip group, timestamps, codec/version, latent scaling, consent/rights record, and split with every sample. Keep benchmark material out of training.

| Source | Purpose | Adapter status / work needed |
|---|---|---|
| FineWeb-Edu — https://huggingface.co/datasets/HuggingFaceFW/fineweb-edu | Educational text | HF text streaming helper implemented; select revision/config and prepare token arrays |
| Panda-70M — https://github.com/snap-research/Panda-70M | Captioned video | Catalog only; rights-aware acquisition, clip cuts, codec caching needed; official license excludes an automatic commercial grant |
| MSR-VTT — https://www.microsoft.com/en-us/research/publication/msr-vtt-a-large-video-description-dataset-for-bridging-video-and-language/ | Video description/retrieval | Catalog only; normalize captions and clips |
| ActivityNet — http://activity-net.org/ | Temporal actions | Catalog only; preserve segment labels and time ranges |
| MusicCaps — https://huggingface.co/datasets/google/MusicCaps | Music descriptions | Catalog only; 5,521 annotated examples, not a massive waveform corpus; linked media availability and rights are separate |
| AudioSet — https://research.google.com/audioset/ | Sound-event ontology | Catalog only; annotation labels are not a guarantee of waveform redistribution rights |
| DataTrove — https://github.com/huggingface/datatrove | Distributed preparation | Upstream integration not installed; replace single-process exact dedup with sharded extraction/filtering/MinHash and global split checks |

## Official evaluation engines

| Engine | Measures | Execution boundary |
|---|---|---|
| SWE-bench — https://github.com/SWE-bench/SWE-bench | Repository issue resolution with tests | Run official container harness in isolated disposable workers, with no secrets or host socket; import results |
| MM-Vet — https://github.com/yuweihao/MM-Vet | Integrated image-language capabilities | Official data/evaluator and judge setup required; not a general video/music fidelity score |
| VBench — https://github.com/Vchitect/VBench | Video quality dimensions | Install upstream, render benchmark prompt suite, execute official evaluator; retain per-dimension results |
| FAD — https://github.com/microsoft/fadtk | Audio embedding distribution difference | Choose embeddings/reference set, extract consistent embeddings, assess sample-size effects |
| G-Eval — https://arxiv.org/abs/2303.16634 | Rubric-based model judgments | Requires a judge model and calibration; does not replace grounded factual verification |
| BLEU / ROUGE | Text overlap | Implemented through SacreBLEU and rouge-score; report tokenization and dataset aggregation policy; current CLI reports mean sentence metrics |

Exam sets should contain ID, source, license, question, choices, answer, subject, difficulty, split, and contamination group. Score held-out exact accuracy with uncertainty; never convert synthetic practice-question accuracy into an official SAT/GRE/GMAT score.

RAG records should retain query, retrieved passages with IDs, answer, reference, claim-level support labels, and judge provenance. The current CLI aggregates supplied judgments. An automatic judge must be independently validated, pinned, and checked for prompt injection and position bias.

Model selection should use per-task release gates and matched data/compute budgets. Do not add arbitrary scores into a single 'intelligence percentage'. Loss values from different modalities have different scales; monitor gradient contributions and tune weights on held-out performance.
