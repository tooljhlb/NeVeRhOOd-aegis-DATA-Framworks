# Large artifact transport

The 256 MiB worker bundle, worker-return and project-export limits remain in
place. They bound control-plane operations. They are not the intended path for
5 GB shards or complete model bodies. Do not split a model arbitrarily through
worker RPC or remove its limits to make large bodies fit.

A separate native filesystem-store adapter provides:

```sh
aegis-data artifact-pack shard-00001.bin /source/chunks shard.manifest.json --chunk-mib 8
aegis-data artifact-transfer shard.manifest.json /source/chunks /destination/chunks --manifest-sha256 MANIFEST_HASH
aegis-data artifact-materialize shard.manifest.json /destination/chunks shard-restored.bin --manifest-sha256 MANIFEST_HASH
```

Retain the manifest hash from `artifact-pack` through a trusted channel. Every
chunk has a SHA256 identity. Packing reuses identical verified chunks; transfer
skips verified destination chunks; interrupted transfers resume at chunk
boundaries. Materialization verifies an existing `.partial` prefix, discards an
incomplete last chunk, resumes, verifies the whole object, and publishes without
overwriting an existing destination. A mismatched complete output or corrupt
partial prefix fails. Concurrent materializers use an advisory Unix file lock.

Chunk sizes are 1–64 MiB (8 MiB default); a manifest permits at most one million
chunks. This allows 5 GB and larger files without allocating the whole file.
Disk capacity, available filesystem sizes and permissions still apply. The
manifest is published only after all source chunks were processed. Sources and
stores must be immutable while in use; callers must own the destination paths.
Temporary `.upload-*` files left by hard crashes are not treated as valid chunks;
clean them only when no producer owns the store. There is no automatic eviction.

The adapter operates between accessible filesystem directories. It can be used
with an administrator-provided mounted filesystem; it does not implement public
HTTP, S3/R2/HF credentials, remote host provisioning or verified mount security.
A future HTTP/object-store adapter should reuse the same chunk/manifest contract
with scoped auth, expiry, cancellation, bounded parallel requests and retries.
Local-store tests do not certify independent-host deployment.

For a body consisting of many shards, retain its format-specific index and an
ordered model manifest that binds each shard hash. This transport moves bytes;
it does not prove tensor correctness, optimizer compatibility, tokenizer identity
or safe deserialization. Never load an untrusted executable checkpoint merely
because its hash matches. Actual codec/load/resume checks belong to the model's
validated launch/checkpoint contract.
