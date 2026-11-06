# Windows Rust candidate · 2026-11-06

Windows x64 unsigned NSIS candidate. Apple-native releases remain unchanged; no Rust macOS/iOS product is released.

## Fixed and verified

- Windows existing text-output replacement: preserve source/target identity and byte checks, release the app's captured destination handle immediately before the existing write-through move. Changed targets, source links and external blocking handles still fail safely.
- Exact-source MSVC: 1,076 Rust tests, 12 notice tests, strict Clippy/formatting, release WASM, three core smoke checks, NSIS structure and bundled-notice validation passed.
- Same installer, Windows Server 2025 x64: actual native Open, rendering/navigation, range-dialog cancellation, shape draw/save/reopen/select, four UTF-8 text exports, native Save cancellation and confirmed overwrite, two-page PNG export, recent/favorite deduplication, Ink draw/whole erase/Undo/Redo. No generic parameter card in these tested workflows; no WebView errors. Owned original PDF unchanged. Exported bytes and saved PDFs independently read back.
- Runtime-only QA corrections reused the exact installer; no second product build or skipped failed assertion. Earlier failed QA runs are not presented as passes.

## Identity

Product source snapshot: `d1c52d774ad1efee2d262ac1824b965537083e2c`, 289 audited Rust/workflow files, byte-identical in the public mirror. Public-only records and publication audit are outside that product snapshot. Root `SOURCE-MANIFEST.sha256` covers the public source tree.

| File | Bytes | SHA-256 |
| --- | ---: | --- |
| `pdfmaster-rust-candidate-windows-x64-nsis.exe` | 33304266 | `f753d646df9a836e620c3ebd8c47b7220a6dbd2206f255d6113cc4d02c3c0332` |

Installed `pdfmaster-app.exe` SHA-256: `d21ea7490e2bef3c8546095221f010eb83014f108738b1a369371f9d762958ba`.

## Limits

This is a development candidate, **not complete feature/UI parity**. Unsupported tools remain unavailable; text-markup selection and paired original-app visual acceptance are incomplete. Hosted Windows Server x64 is not clean consumer Windows 10/11 or human acceptance. No code-signing/SmartScreen assurance. Never disable global OS security to install. Keep backups and use copies of important documents while evaluating.

No private history, QA profiles, REA resources, credentials, user PDFs or private evidence is published. Existing copyright and third-party license/notice terms remain applicable; public source sharing does not introduce a new blanket software license.
