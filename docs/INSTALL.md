# Linux runtime installation and project workflow

The GitHub verification workflow builds a release binary and uploads an
`aegis-linux-x86_64-runtime` artifact. Download it from the workflow run for the
reviewed commit, extract it to a new directory, review `SOURCE_REVISION` and
`SHA256SUMS`, then run:

```sh
bash /path/to/package/install.sh "$HOME/aegis-runtime-v1"
"$HOME/aegis-runtime-v1/bin/aegis-data" --help
```

The installer validates fixed package membership hashes and platform metadata.
It refuses to overwrite an existing installation directory. Use versioned
installation directories for upgrades; retain the previous version for rollback.
The package needs Bash, GNU coreutils, a compatible Linux/glibc environment, and
any workload toolchains separately installed. It does not bundle compilers,
Docker, models, Python, GPU drivers or provider credentials. Current CI verifies
Ubuntu x86_64; other Linux distributions and architectures remain unverified.
Checksums establish byte integrity, not publisher authenticity. Obtain packages
from the reviewed GitHub workflow; there is no signing or automatic updater yet.

To package a locally built runtime:

```sh
cargo build --release --locked
bash scripts/package.sh target/release/aegis-data /tmp/aegis-package FULL_GIT_COMMIT_SHA
```

The revision argument labels the package; CI supplies the actual checked-out
`GITHUB_SHA`. This is not a claim of byte-for-byte reproducible binaries.

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

CI tests the installed binary outside its package directory against the
repository's actual C++ reference component, including intentional compilation
failure, corrected execution, export verification and tamper rejection. This
covers a repository component; independent projects and users remain separate
acceptance requirements.
