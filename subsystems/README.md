# Optional subsystems

The framework includes an optional editor integration at [editor/](editor/). The Rust-native local workspace remains available without it.

- Monaco browser integration is built separately with Node tools.
- Its pinned dependency manifest and third-party license notices are included.
- Source-language integration uses bounded backend LSP queries; a complete IDE/debugger and extension host are not yet implemented.
- Optional dependencies are never proof that a runtime, model, or external provider has been installed.

The public edition does not redistribute private source archives, imported upstream snapshots, external account configurations or internal experimental records.

For usage, see [the editor guide](../docs/EDITOR.md).
