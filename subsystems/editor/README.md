# Optional local AEGIS Monaco client

This client uses Monaco 0.57.0 pinned by npm lock/integrity, served from a hash-pinned
local bundle by Rust. No Node server, CDN, hosted model or API key is needed at
runtime. This is browser integration code; core execution remains native Rust.
Upstream Monaco copyright/license is retained under `licenses/` and in the bundle.
The uploaded TypeFox monaco-languageclient is separately tracked in the upstream
lock; it has NOT been integrated as a persistent language-client session here.

## Build and serve

```sh
npm --prefix subsystems/editor ci --ignore-scripts --no-audit --no-fund
npm --prefix subsystems/editor run build -- /absolute/new/monaco-assets
```

Use a fresh output directory. The build prints the SHA256 of `manifest.json`.
Configure the operator with:

```sh
export AEGIS_EDITOR_ASSETS=/absolute/new/monaco-assets
export AEGIS_EDITOR_ASSETS_MANIFEST_SHA256=PRINTED_MANIFEST_SHA256
# Existing AEGIS_OPERATOR_TOKEN and optional AEGIS_LANGUAGE_SERVERS also apply.
aegis-data operator-serve ledger.db /repository --trusted-local
```

Open `/workspace`, authenticate, select a file, and enable local Monaco. Plain-text
mode remains available. Each file has its own Monaco model, syntax highlighting
and the existing revision-aware save/snapshot contract. Unsaved buffers still do
not survive reload; no persistence claim is added by the editor library.

Enter an operator-configured language-server ID and analyze a saved revision. The
native backend runs its bounded LSP diagnostics query. The client checks source
hash, buffer identity and dirty state before displaying results/markers. Editing
clears stale markers. Unsaved buffers must be saved before analysis. Diagnostics
are not incremental, automatic, full-workspace certification or a complete
IntelliSense implementation. Completion, rename, DAP and persistent language-client
sessions remain separate work. See `docs/AI_BACKEND.md` for backend limitations.

The Rust server validates the trusted manifest hash and every declared asset's
size/hash at startup, rejects symlinks and unsupported file types, then serves only
those immutable in-memory assets with correct MIME and nosniff. Limits: 1,024 files,
20 MiB/file, 64 MiB total, 2 MiB manifest. Unknown assets return 404. Static assets
are public like `/workspace`; workspace data and LSP queries require bearer auth.
No live directory listing or arbitrary filesystem static route is exposed.

Build output has lazy language chunks and a local editor worker, CSS and notices.
Core package installation is unchanged. CI publishes frontend assets separately;
verify and pin the artifact manifest before enabling it. `npm ci` is development-
time package retrieval, not inference traffic. Fully offline builds require an
already populated dependency cache or a separately verified dependency mirror.

`verify_monaco.py` launches Chromium, loads the locally built editor, renders real
Ruff F821 diagnostics, edits/saves/reanalyzes a file, toggles plain-text mode, checks
for external browser requests, and tests raw binary packet CLI/API round trips.
This is a small local integration test, not full IDE or multi-user certification.
