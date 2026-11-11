# Linux source build and runtime installation

This public edition currently provides **source code and build/packaging scripts**, not a prebuilt binary download. The commands below run on a Linux x86_64 development machine with stable Rust/Cargo, Bash and GNU coreutils.

```sh
cargo test --workspace --locked
cargo build --release --locked
target/release/aegis-data --help
```

To create a version-labeled runtime package locally:

```sh
bash scripts/package.sh target/release/aegis-data /tmp/aegis-package "$(git rev-parse HEAD)"
bash /tmp/aegis-package/install.sh "$HOME/aegis-runtime-v1"
"$HOME/aegis-runtime-v1/bin/aegis-data" --help
```

The installer verifies the packaged files using SHA-256 checksums and refuses to overwrite an existing installation directory. Preserve versioned install directories for rollback. Checksums establish byte integrity, not publisher authenticity. A local package does not imply an independently attested, signed or reproducible release.

The runtime package does not bundle compilers, Docker, Python, model weights, drivers, toolchains or credentials. Installation on hosts outside Linux x86_64 has not been validated in this public release. The included `scripts/verify_package.py` contains packaging/integration checks, but the new public repository must obtain its own successful verification evidence.

## Run an existing project

Write an operator plan with its actual workspace, build/test commands, dependency
order, declared input files and expected output files. Use a runtime profile
allowing only the required commands and budgets; see [RUNTIME.md](RUNTIME.md).
The program and args are passed separately, without implicit shell evaluation.

```sh
aegis-data project-doctor plan.json
aegis-data runtime-check profile.json plan.json
aegis-data project-run profile.json plan.json operator.db /tmp/project-evidence --trusted-local
aegis-data project-verify /tmp/project-evidence --manifest-sha256 MANIFEST_HASH_FROM_PROJECT_RUN
```

Doctor checks plan validity, workspace existence and executable file identities.
A program produced by an earlier build stage is unavailable until that stage has
run; doctor reports this rather than assuming a tool exists. It does not certify
Docker daemon access, compiler compatibility, drivers or available disk space.

`project-run` uses the existing durable operator. Failed checks withhold export.
Inspect `operator-status DB RUN_ID` and `DB.artifacts/RUN_ID/` for logs. Existing
attempt budgets and resume contracts apply. After changing inputs or consuming
an attempt budget, use a new run ID for a corrected experiment. A completed run
can be reused only while its declared inputs, outputs and executable identities
still match. No automatic repair is implied by this command; the separate case
workflow provides rehearsal, acceptance and rollback.

Export includes declared input/output files, logs, status, receipt metadata and
recorded run events. It validates artifact and executable identities and the
source event ledger before copying. Symlinks and special files are rejected.
Packages are bounded to 256 MiB of payload/logs, 10,000 files and at most 1,000
run events / 16 MiB of event payload. Larger runs require a separate shard or
checkpoint transport; export fails rather than silently truncating events.

`project-verify` requires the separately retained manifest hash, checks exact
file membership and hashes, and rejects added, missing or modified files. It
establishes anchored byte integrity, not independent re-execution or a portable
machine image. The source ledger was verified at export; `events.json` is a
bounded event report, not an independently verifiable complete ledger database.
Exported commands, paths and source may contain project information: review the
package before sharing. Exports are local until explicitly distributed.

The package-verification script exercises installation and local project workflows, including intentional failure, correction, export verification and tamper rejection. Run it in a supported environment and retain revision-specific receipts before claiming this public release has passed. Independent projects and users remain separate acceptance requirements.
