# Local editor workspace

Start the existing authenticated loopback operator server with its workspace root
set to the repository to edit, then open `/workspace`. The cockpit links to it.
The token stays in page memory; navigating to another page requires re-entry.

The workspace provides a source file tree, plain UTF-8 tabs, literal text search,
revision-aware save, snapshot restore, Git status/working-tree diff and a bounded
command panel. Commands use the existing operator API, policy, run IDs, logs,
timeouts and cancellation. They are not an interactive terminal or PTY. Git views
use bounded native execution, disable external diffs/textconv and filesystem
monitor hooks, and report failures or truncated output explicitly.

## Editing contract

GET `/v1/workspace/file?path=relative/path` returns text and SHA256. POST to the
same route with `{path, expected_sha256, text}` writes only if the loaded preimage
still matches. A stale revision returns HTTP 409. POST
`{path, expected_sha256, snapshot}` restores a previously retained content hash.
The browser retains its unsaved buffer after a rejected save. Reload/rollback
asks before discarding a dirty buffer; unsaved content does not survive a page
reload, crash or device loss. A before-unload prompt is best-effort browser UI.

Saves use an advisory workspace lock shared by cooperating API/CLI processes,
preimage checks, original permission preservation, a synchronized replacement
file, atomic rename and snapshots/journals under `.aegis/editor`. A completion
write can fail after source replacement; pending journals and snapshots remain
for inspection. This is single-file recovery, not a crash-certified multi-file
transaction. Disk-full and power-loss fault injection remain unverified.

Unrelated editors and processes can ignore the advisory lock. The final preimage
check reduces accidental external conflicts but is not an OS compare-and-swap
against uncontrolled external writers. Use exclusive workspace ownership today;
per-task worktrees, overlap detection and multi-file promotion remain required
for stronger concurrent-agent guarantees. Direct owner editing is distinct from
the protected-test case promotion workflow; it does not enforce agent roles.

CLI equivalents:

```sh
aegis-data editor-tree /repository
aegis-data editor-read /repository src/main.rs
aegis-data editor-save /repository src/main.rs /tmp/replacement.rs --expected-sha256 LOADED_HASH --trusted-local
aegis-data editor-rollback /repository src/main.rs SNAPSHOT_HASH --expected-sha256 CURRENT_HASH --trusted-local
```

Internal Rust callers use the same `editor` module. Read/search/Git APIs require
operator authentication. Paths stay under the server root; traversal, symlinks,
Git metadata, common generated directories and secret-like paths are denied.
This conservative filename policy is not a complete secret detector. Files are
existing regular UTF-8 text, limited to 128 KiB. File creation/deletion, binary
editing and large-file editing are not implemented. The file tree is bounded to
depth 8, 20,000 visited entries and 5,000 files. Search inspects at most 16 MiB and
returns at most 200 literal substring matches. Snapshot retention/cleanup remains
manual; this is trusted-local storage, not a multi-tenant vault.

## Product scope

This is a functioning local plain-text editor foundation, not full IDE parity.
LSP diagnostics/completion/navigation, DAP debugging, persistent unsaved buffers,
interactive terminals, extension compatibility, durable agent objectives/context,
per-task worktrees and team authorization remain separate acceptance work.
No model inference adapter or streaming chat endpoint is added by this editor.
Hosted service isolation and identity cannot be inferred from loopback auth.

## Optional Monaco client

A separately built, hash-pinned local Monaco bundle can now replace the textarea
per page, preserving the revision-save backend and plain-text fallback. Saved-file
LSP diagnostics render as markers after source identity checks. See
[local client setup](../subsystems/editor/README.md). Full monaco-languageclient
sessions, DAP and multi-file transactions remain unfinished.
