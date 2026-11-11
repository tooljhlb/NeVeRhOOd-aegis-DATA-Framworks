# Usefulness measurements

Readiness is an acceptance backlog. Operational usefulness needs separate data.
`outcome-report observations.jsonl report.json` aggregates caller-recorded
observations, retaining input hash, sample counts and missing values.

Each JSONL record has a unique `run_id` and optional integer measurements:
`time_to_first_success_ms`, `recovery_ms`, `unnecessary_reruns`, `patch_attempts`,
`patch_regressions`, `operator_interventions`. Omitted fields remain unmeasured.
Patch attempts/regressions must occur together, and regressions cannot exceed
attempts. The report provides nearest-rank p50/p95 and observed sample counts;
regression rate is total observed regressions / attempts, or null with no attempts.

Define start/end conditions before collecting observations: first success starts
at an agreed onboarding point; recovery starts at detected failure and ends at
verified restored output. Count a rerun as unnecessary only after recording why
it was redundant. Count operator intervention consistently (for example explicit
manual changes or decisions); do not infer independence from raw chat turns.
Record follow-up test failures as patch regressions using a fixed test window.

These measurements are recorded by the caller, not automatically or independently
attested. Do not mix fixture timings with customer outcomes or treat missing
observations as zero. Compare cohorts with matched workload, hardware, policy,
revision and operator experience; these percentiles have no uncertainty estimate.
