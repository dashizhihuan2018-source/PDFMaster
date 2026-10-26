# PDF Master Rust — migration candidate

This is an **in-progress full rewrite**, not the installed PDF Master replacement and not a feature-complete release. Business logic and UI are authored in Rust. Leptos compiles the UI to WASM; Tauri uses platform WebViews, generated JS glue and system SDKs. Those dependencies are not falsely described as Rust implementations.

## Structure

- `crates/pdfmaster-core`: transactional PDF operations and rendering via lopdf / hayro. Unsupported cases fail explicitly; complex geometry, Unicode content editing and several resource classes remain incomplete. It is **not yet approved for arbitrary untrusted PDFs**.
- `app`: Leptos Rust UI; no handwritten JavaScript application frontend.
- `src-tauri`: native dialogs, guarded desktop lifecycle, private local draft journal and no-clobber desktop export. A Rust UIKit adapter stages the complete PDF before iOS export and verifies provider readback; local simulator cancel/save/reopen has been exercised. Remote providers and physical devices remain unverified.
- `cross-platform/parity.json`: 801 legacy XCTest method contracts; all three platforms remain pending until actual ported-contract evidence is attached. This is not a complete UX inventory and not a test pass report.
- `tools/baseline.rs`: read-only legacy contract extractor using Rust std.

## Pinned toolchain

Rust 1.96.0, Tauri CLI 2.12.4, Trunk 0.21.14. Cargo.lock files are committed for the independent core, UI and native workspaces. Use rustup's cargo rather than an unrelated system cargo. Set `NO_COLOR=true`; Trunk rejects the host convention `NO_COLOR=1`.

```sh
rustup target add wasm32-unknown-unknown aarch64-apple-ios-sim aarch64-apple-ios
cargo install trunk --version 0.21.14 --locked
cargo install tauri-cli --version 2.12.4 --locked
cargo test -p pdfmaster-core
cargo test --manifest-path app/Cargo.toml --lib
cargo test --manifest-path src-tauri/Cargo.toml --lib --test security_contract
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
NO_COLOR=true cargo tauri build --debug --bundles app
```

On Apple Silicon, the installed `cargo-tauri` **must itself be arm64**. Installing under Rosetta generates Intel simulator settings that can persist even after reinstalling the CLI. Preserve an old generated project separately, then regenerate from the native CLI; do not delete source or user data. Xcode, CocoaPods, xcodegen and simulator runtimes are prerequisites.

```sh
NO_COLOR=true cargo tauri ios init --ci --skip-targets-install
NO_COLOR=true cargo tauri ios build --debug --target aarch64-sim --ci --no-sign --archive-only
```

An unsigned simulator archive is not an iPhone IPA, App Store release, TestFlight deployment or physical-device acceptance. Windows native build/run and package verification remain pending; do not interpret successful macOS or target checks as Windows support.

## Data safety / release boundaries

- Edits operate on session bytes. Desktop export creates a new PDF without overwriting an existing path. iOS delegates destination selection/conflict handling to the system provider. Both validate parse and exact readback before clearing dirty state; original source files are not implicitly overwritten.
- Drafts stay in the candidate's private local app-data directory, with atomic replacement, 0600 files / 0700 directory on Unix, and an exclusive process lock. No cloud draft sync is implemented. Corrupt drafts are retained and block unsafe replacement.
- Normal dirty close/quit is blocked. Native macOS default Quit bypasses Tauri exit events, so the custom menu explicitly routes through `app.exit`. Unexpected OS termination is covered by the local draft journal, not by a claim that termination is impossible.
- The note-content editor acquires a snapshot-bound native session before exposing input. Normal close/quit and conflicting edits are guarded until explicit save/cancel. If the begin reply is lost, “同步便笺编辑状态” can recover the active session's original contents and ID in the current process; it cannot recover unpersisted typed text. Editor input is not crash-persistent, and the new guard's actual desktop/mobile interaction verification remains pending.
- Export reserves the workspace from snapshot capture through the picker and target readback, without holding its mutex across provider I/O. Other mutations cannot race that snapshot; cancellation/error releases only the matching reservation. Unit tests do not replace native picker and asynchronous cancellation integration tests.
- Do not commit `target/`, `dist/`, generated native projects, private draft data, user PDFs, signing material, credentials or personal screenshots.
- Do not install this candidate over the formal application or publish it as full parity. Final delivery requires actual three-platform evidence, independent review, corresponding clean public source, package hashes and permitted signing/distribution.
