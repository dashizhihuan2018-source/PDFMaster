# Candidate source snapshot — not a product release

2026-10-26. Additive Rust migration source imported from development revision `b7a656a4` using an explicit tracked-file allowlist (`RustPDFMaster/` and this candidate workflow only). Existing native product source, installer, version tags and Releases are not replaced by this snapshot. No private development history, drafts, personal PDFs, credentials, generated project, build cache or screenshots are included.

The Rust core and UI are intentionally incomplete. Full parity is not approved. Current locally exercised foundation: 77 core tests, 11 UI tests, 23 native-shell tests; 10 CI/artifact contracts; 4 inventory contracts. The 801 original XCTest traceability entries remain pending, not passing Rust contracts. Synthetic PE fixtures validate the parser, not Windows installation. macOS and iOS simulator self-created PDF workflows were exercised locally; physical iOS, Windows runtime, signing, notarization, App Store and complete product acceptance remain pending.

The manually dispatched workflow builds unsigned, clearly named candidate artifacts on standard GitHub runners. It has read-only repository permissions and does not publish a Release. A successful build is not install/run/UI parity evidence. Existing formal PDF Master remains the user's installed application.

Business logic and application UI are authored in Rust. Platform WebViews/system SDKs, HTML/CSS, generated JavaScript glue, and CI Bash/Python are disclosed rather than called pure Rust. `Cargo.lock` records exact dependency resolutions. Research reference projects are not copied into this source.
