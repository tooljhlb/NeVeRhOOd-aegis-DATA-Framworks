# Eight uses, shared execution and evidence

`aegis-data use-cases` lists eight runnable use paths. The authenticated local API exposes the same catalogue at `/v1/use-cases`; the cockpit provides a selector. These are bounded first implementations with explicit scopes, not eight finished enterprise products.

| Use | Working path | Current boundary |
|---|---|---|
| Software flight recorder | `case-capture` → `case-rehearse` | Explicit source selection, checks, runtime/tool identities and bounded output; not a machine image or complete environment capture |
| Model/vendor comparison | `lab-run` → `lab-compare` | External executable adapter or saved predictions; exact JSON match or numeric MSE, not automatic semantic judging or provider billing |
| Upgrade rehearsal | Case with `purpose: upgrade` | Same declared checks before and after a candidate text change; no automatic dependency manager or database migration engine |
| Source correction impact | `lineage-link` → `lineage-quarantine` → `lineage-check` | Recorded dependency impact and opt-in promotion block; not data deletion or model unlearning |
| Reproducibility exchange | `case-export` → `case-replay` | Hash-anchored portable source/check/patch bundle; toolchains and images provisioned separately |
| Bounded maintenance | `case-propose` → `case-rehearse` → `case-accept-remote` → `case-promote` | Executable proposer contract, protected files, optional required remote OCI, single-file promotion/rollback; no embedded general repair model |
| Compute admission broker | `broker-quote` → `worker-enqueue --worker` | Fresh supplied inventory, resource/capability/image/cost filtering and enforced worker identity; no live reservation, cloud provisioning or billing |
| Incident rehearsal | `incident-drill` | Actual contained worker-loss/disconnect engineering drills; not a production chaos agent |

## Capture, reproduce, repair and replay

A case specification declares source file selection, editable/protected files, exact test commands and expected baseline exit codes. Paths must stay inside the selected source root. Files are regular, bounded and hash-recorded; symlinks and traversal are rejected. This avoids silently crawling private directories. The bounded v1 limits are 256 MiB / 10,000 selected files, 100 checks, 1–3,600 seconds per check, a declared combined before/after timeout budget of at most one day, and 1,000 text-file edits per proposal.

```sh
# From the repository root, with the native binary on PATH:
aegis-data case-capture fixtures/maintenance-code examples/maintenance-case.json /tmp/aegis-case
aegis-data case-rehearse /tmp/aegis-case examples/maintenance-patch.json --trusted-local
aegis-data case-export /tmp/aegis-case /tmp/aegis-exchange
# Supply the exchange_sha256 printed by case-export:
aegis-data case-replay /tmp/aegis-exchange /tmp/aegis-replay \
  --exchange-sha256 EXCHANGE_SHA256 --trusted-local
```

The bundled shell arithmetic project is an intentionally broken **engineering fixture**, not an independently discovered user defect or a general coding benchmark. Four checks cover positive, negative and zero cases. The candidate corrects subtraction to addition. Independent user deployment and effectiveness remain unverified acceptance requirements.

For a real repository, list the relevant source and test files explicitly, declare the actual build/test commands, and protect the tests. Generated build files are allowed in the rehearsal workspaces, but the selected input files must stay byte-identical throughout each test phase. Repair acceptance requires at least one expected, observed nonzero baseline exit, all baseline expectations matching, every candidate check exiting zero without timeout/unavailability, stable inputs and stable tool identities. Upgrade cases permit an all-green baseline. Passing selected checks is not proof that every regression is absent.

Without `--trusted-local`, checks require a digest-pinned `container` in the specification. OCI only mounts the rehearsal workspace. Host execution is explicitly trusted. Keep test commands bounded; global disk quotas, VM escape resistance and multi-tenant operation remain open. Interrupted local case directories are preserved for inspection rather than silently reused; create a new case/replay directory for another attempt.

## Proposer and model adapter contract

`case-propose CASE GENERATOR.json` accepts a standard executable `Check` (`id`, `kind`, `program`, `args`). It runs in a separate copied proposal workspace containing `aegis-request.json`, and must write `patch.json`:

```json
{"edits":[{"path":"source.ext","before":"exact old text","after":"replacement text","occurrences":1}]}
```

The proposer cannot authorize protected-file edits, ambiguous preimages, empty/no-op replacements or unlisted files. Source promotion is a different command. `fixtures/maintenance-proposer.py` exercises the protocol with a deterministic rule; it is **not an AI model**. Supply an explicitly authorized local model/agent executable for model-generated proposals. No hosted provider or hidden model capability is inferred from the fixture. Images/model weights and their licensing remain the operator's explicit inputs.

`lab-run DATASET.jsonl ADAPTER.json OUT --trusted-local` supports `{dataset}` and `{predictions}` argument placeholders. The adapter writes JSONL `{"id":"sample","prediction":...}` rows. The dataset supplies `{"id":"sample","group":"document-family","expected":...}`. Adapters may call a local model or user-approved service; credentials are not automatically forwarded into child workloads. Use isolated operator stages for untrusted adapters.

`lab-compare DATASET BASELINE CANDIDATE --mode exact|regression --out REPORT` rejects missing, duplicate and extra prediction IDs. It reports paired wins/losses, exact-match accuracy or MSE, and a deterministic 500-replicate whole-group bootstrap of candidate-minus-baseline. Fewer than two groups produces no interval. Group independence is caller-declared. Runtime receipts measure executable wall time; real provider costs, opaque weights and model identities need separate evidence. Changing the adapter is not by itself a controlled model comparison.

## Remote isolated verification and promotion

Set `require_remote: true` in the captured case policy when promotion must require remote verification. Enqueue **only the selected candidate input files** as a bundle. The remote plan must run every captured check with the same stage ID/program/arguments, a pinned OCI container and declared outputs. The remote bundle's file hashes must exactly match the reviewed candidate; extra files are rejected by the case verifier.

```sh
aegis-data worker-enqueue remote.db remote-plan.json candidate-bundle/ --worker worker-01
# Run the authenticated worker controller/client as documented in REMOTE.md.
aegis-data case-accept-remote CASE remote.db JOB
# Bind promotion to the reviewed report hash printed by case-rehearse:
aegis-data case-promote CASE PROJECT --report-sha256 REPORT_SHA256 \
  --remote-db remote.db --remote-job JOB --lineage-db lineage.db
aegis-data case-promote CASE PROJECT --report-sha256 REPORT_SHA256 --rollback
```

`case-accept-remote` rechecks controller-stored return hashes, exact candidate inputs and check identities, and records the remote plan digest and image limits in its result. It is trusted-worker evidence, not hardware attestation. The remote worker API never gains permission to promote files on the controller/project.

Promotion rechecks the captured source preimage, accepted report, patch and candidate hashes. Automatic promotion is deliberately limited to **one changed file**. Multi-file candidates can be rehearsed and exported, but require a separate transactional integration mechanism. A pending journal is written before replacement, the file is atomically replaced, and a completed journal follows. If interrupted, compare the source hash with the journal/original bundle for explicit recovery; there is no automatic multi-file crash transaction. Preserve exclusive project ownership during promotion; external editors do not participate in an OS-wide compare-and-swap protocol.

Rollback checks the candidate preimage and restores the retained original. It remains available after source quarantine so an operator can undo a prior promotion. Git commit, PR approval and deployment are separate integration steps.

## Source correction and dependency impact

```sh
aegis-data lineage-link lineage.db CASE/report.json source-file test-file CASE/patch.json
aegis-data lineage-quarantine lineage.db SOURCE_SHA256 'Source needs correction'
aegis-data lineage-check lineage.db REPORT_SHA256
```

These commands use the append-only graph. Quarantine follows recorded `derived_from`/`uses`/`depends_on` relationships and marks affected artifacts. Re-registering the same hash does not clear quarantine. Passing `--lineage-db` to promotion requires the report and its recorded dependencies to be allowed. This gate is opt-in; it is not silently enforced by unrelated export/training tools. Missing lineage is rejected by that gate, not assumed clean. A corrected source gets a new content hash and new experiment; no automatic deletion or unlearning is claimed.

## Admission quotes and worker targeting

`broker-quote REQUEST.json INVENTORY.json --out QUOTE.json` consumes resource requirements (`cpu`, `memory_mib`, `gpu_count`, `gpu_memory_mib`), `duration_seconds`, integer `max_cost_micros`, required `capabilities`, optional pinned `image`, and `max_age_seconds` (1–3,600).

Inventory is an array with `id`, `observed_at_unix`, available resource fields, integer `price_micros_per_second`, `capabilities` and `images`. Stale/future inventory, insufficient resources, missing capabilities/images and excess estimated cost are rejected. Eligible workers sort by estimated cost, then stable ID. Inventory is supplied evidence, not independently measured live capacity. Quotes do not reserve capacity; concurrent quotes can overcommit unless a provider/scheduler adds reservation.

`worker-enqueue --worker ID` persists that identity requirement in the immutable payload. Both local and remote claims enforce it transactionally; another worker cannot take the job. Enqueue the chosen ID only after reviewing the quote and provisioning the actual worker/image. This is admission/placement plumbing, not a complete cloud compute marketplace.

## Contained incident drills

```sh
aegis-data incident-drill . target/debug/aegis-data /tmp/worker-drill \
  --scenario worker-loss --python python3 --trusted-local
aegis-data incident-drill . target/debug/aegis-data /tmp/remote-drill \
  --scenario remote-disconnect --python python3 --trusted-local
```

The native launcher runs the existing executable fault-injection suites in temporary fixtures and records script/binary identities, observations and logs. They kill actual fixture processes, test lease recovery and cancellation, and leave production resources alone. These verification adapters use Python; the controller, worker, case engine, scoring and provenance/admission logic are Rust.

## Verification

`scripts/verify_uses.py` exercises semantic failure/candidate checks, executable proposal contract, protected-test rejection, baseline mismatch rejection, independent exchange replay/tamper rejection, promotion/rollback, changed-source rejection, source quarantine, upgrade rehearsal, actual prediction adapter and grouped comparison, regression MSE, admission filtering and contained fault drills. `--image DIGEST_PINNED_IMAGE` adds the complete authenticated remote OCI repair and policy-gated promotion. CI runs that mode on a Docker-capable runner. Independent people, external model repair quality, provider billing and production-scale behavior still require separate evidence.
