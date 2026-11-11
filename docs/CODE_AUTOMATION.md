# Polyglot preparation and automated checks

## Coverage model

`configs/languages.json` is a versioned registry of 42 language/tooling profiles: Rust, Python, JavaScript, TypeScript, JSX, C, C++, C/C++ headers, CUDA, Go, Java, Kotlin, C#, F#, Swift, Objective-C, Ruby, PHP, shell, PowerShell, Lua, R, Julia, Scala, Dart, Elixir, Erlang, Haskell, OCaml, Zig, SQL, HTML, CSS/SCSS/Less, JSON, YAML, TOML, Protobuf, FlatBuffers, Terraform, Dockerfile and CMake.

These are command templates, not 42 installed or certified toolchains. Extend `extensions`, `filenames` and `checks` for any other language. A check has an ID, kind (`lint`, `format`, `syntax`, `type`, `test`), executable, argument array, and optional project marker. `{file}` is substituted as one argument with a relative `./` prefix; no shell interpolation occurs. Project-scoped checks run once at the scan root. For monorepos, invoke each project root with its own registry/config. No package manager is automatically allowed to download linters or dependencies.

Language recognition does not prove correctness, executable success, security, or model capability. The current CI profile verifies Rust formatting/Clippy, Python Ruff, C++ syntax, C++/CUDA formatting, ShellCheck, and JSON/YAML parsing. CUDA formatting is not CUDA compilation or GPU execution. Other templates require the appropriate tools, project configuration and local verification (for example SQL dialect, compiler include paths, ESLint config or Checkstyle config).

## Commands

```bash
cargo run --locked -- code-scan . --out /tmp/inventory.json
cargo run --locked -- code-check . --trusted-local --out /tmp/all-checks.json
cargo run --locked -- code-check . --languages rust,python,cpp,cpp-header,cuda,shell,json,yaml --trusted-local --out /tmp/selected-checks.json
```

`code-scan` is read-only, respects ignore rules, excludes common generated/vendor/build folders, does not follow symlinks, and hashes recognized source files up to 2 MiB. Oversize recognized files are recorded and prevent a complete report. Unrecognized extensions are not evaluated; add a registry entry to cover them. Empty check suites and missing tools cannot produce a complete report. No `--fix` command is run automatically.

`code-check` uses bounded per-stream output capture (64 KiB stdout and 64 KiB stderr), deadlines, and Unix process-group cancellation. Results include each command, source/registry identities, status, exit code, duration and output truncation. Missing tools, failed checks, timeouts and absent project configuration make the command fail after writing its report. Explicit `--languages` selection is preserved in the report; unselected profiles are not claimed as tested. An inherited environment allowlist avoids forwarding common API credentials, but tools can still read the trusted user's local configuration.

The command executor currently requires Unix. It is a trusted-local tool runner, not an untrusted-code sandbox. Build systems, test runners and configurable linters can execute code. Run model-generated/untrusted projects in disposable isolated workers. Escaped/daemonized descendants are not contained by Unix process-group cancellation; this is another reason not to use trusted-local execution as a sandbox.

## Continuous automation

The GitHub `quality` job runs on pushes and pull requests, installs pinned lint tool versions, checks the explicitly supported profile, and uploads its machine-readable report even on failure. The native and research jobs independently run mathematical, dataset-integrity, and checkpoint-resume tests. Failures remain visible; lint success cannot replace task execution tests.

To add a test runner for another language, create a `kind: test` check with the project's argument array. Prefer a project marker to prevent running a full suite for each file. Commands that mutate files, install dependencies or access the network need a deliberately configured worker environment; the registry does not provide sandboxing or provisioning.

## Experiment evidence

`compare BASELINE.json CANDIDATE.json --out REPORT.json` validates experiment identities and compares per-domain metrics. It records changed data, base model, tokenizer, objective, schedule, seed or update count as confounders. Metrics with different evaluation data, sample counts, directions or missing counterparts are `not_comparable`. Binomial metrics can carry successes/counts, producing Wilson 95% intervals; generic scalar loss metrics do not receive invented uncertainty. Changes are descriptive, not significance tests or proof of causality. Direction `min` means a minimum threshold (larger is better); `max` means a maximum threshold (smaller is better).

Release gates now require `evidence_file`, `evidence_sha256`, and an RFC 6901 `metric_pointer` into the result JSON. The score is read from those exact verified bytes. Optional legacy `measured` values must agree exactly and cannot override the artifact. Missing, nonnumeric, oversized (>16 MiB), mismatched or tampered evidence fails. A hash and a JSON pointer do not certify that the upstream evaluator or dataset is valid.

## Real-source reference pilot

```bash
pip install -e './research[eval]'
python scripts/run_reference.py . --out /tmp/reference-pilot --license-reference 'Repository LICENSE' --steps 20
```

This pilot scans up to 500 local code documents, records source hashes and license reference, groups exact/high-overlap documents using whole-document five-word shingle Jaccard >=0.8 before split assignment, prepares native records, validates per-shard hashes/counts, exports fixed byte windows and measures untouched validation loss before/after training. It verifies exact CPU model/optimizer/RNG/cursor recovery on a split run.

The immutable source manifest and prepared dataset manifest describe each run. The native tokenizer uses a deterministic hash-priority reservoir capped at 256 rows per split in this pilot, retaining document/group identities. Validation uses a deterministic capped subset, records window and target-token counts, and reports duplicate-group bootstrap intervals; fewer than two groups yields no interval. This is a small same-repository language-loss experiment, not a software-task benchmark, a production data pipeline, or a matched-compute architecture comparison. No external benchmark contamination audit or semantic deduplication is claimed. A corpus with no complete validation windows fails explicitly.

## Current preservation/reproducibility contract

Code is preserved exactly by both native and reference preparation. Prose normalization is optional and recorded. Prepared shards use schema v2. Experiments require evaluator-code, evaluator-config and decoding hashes; mismatches make metric pairs incomparable. Checkpoint computation identities bind configuration, code, objective, optimizer, precision and runtime. The native operator now provides the complete local workflow documented in `docs/OPERATOR.md`; older roadmap descriptions of these slices as entirely missing are superseded.
