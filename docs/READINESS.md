# Public acceptance coverage

The `readiness.json` registry contains 20 acceptance areas and 84 checks. In this standalone public edition, all inherited evidence references are intentionally **unset**, rather than relying on receipts tied to a private development history.

```sh
cargo run --locked -- readiness readiness.json --root . --out /tmp/readiness-report.json
```

A check with `evidence: null` is **unverified**, not failed or passed. Each future public receipt must identify a public relative JSON artifact, SHA-256 digest, JSON pointer and expected value at a reviewed code revision. A reference hash proves those bytes match; it does not prove benchmark validity, independent reproduction or product usefulness.

The public workflow reruns native tests and reference commands on its own commit. CI state is distinct from the acceptance registry and from actual independent customer outcomes. Do not collapse these into a single quality score.
