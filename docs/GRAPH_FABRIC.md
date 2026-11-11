# Provenance graph and public example catalog

The graph fabric records **observed** run, stage, artifact, source and dependency nodes using native SQLite projections of append-only hash-chained events. Diff, time-indexed snapshots and impact tracing are available through the CLI.

This release bundles a small **hand-authored public example** at `graphs/catalog.json`. These definitions are `declared`, not executable graph engines or claims that neural processing is running.

```bash
cargo run --locked -- graph-import-catalog /tmp/aegis-graphs.db
cargo run --locked -- graph-snapshot /tmp/aegis-graphs.db --out /tmp/aegis-graph.json
cargo run --locked -- graph-verify /tmp/aegis-graphs.db
```

For real observational data, run an operator workflow; stage and artifact relations are recorded as stages execute. A dependency edge represents potential impact, not demonstrated causality. Event hashes detect changes relative to trusted retained hashes; there is no independent external timestamp authority.

To extract a fresh declarative catalog from **your own** specification:

```bash
cargo run --locked -- graph-extract /path/to/public-spec.txt --source-url 'public or local source identity' --out /tmp/public-graph-catalog.json
```