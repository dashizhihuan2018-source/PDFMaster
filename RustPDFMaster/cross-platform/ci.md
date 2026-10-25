# PDF Master Rust candidate CI

## Purpose and boundary

This is a manually dispatched **candidate-build** workflow. It does not publish a Release, push code, install the candidate, sign/notarize packages, or claim macOS/Windows/iOS GUI parity. Product business logic and UI remain Rust; the repository-local Bash/Python files are CI orchestration and artifact validation, not application logic.

The workflow is `.github/workflows/pdfmaster-rust-candidate.yml` and has only `workflow_dispatch`, `contents: read`, bounded concurrency and per-job timeouts. Checkout credentials are not persisted. No workflow secret is referenced.

## Locked tools and native runners

- Rust `1.96.0`, with committed lock files used by core, UI and shell checks.
- Trunk `0.21.14`; the WASM build uses `--locked`, and Tauri builds set `TRUNK_BUILD_LOCKED=true`.
- `tauri-cli 2.12.4`; packaging forwards Cargo `--locked` to the runner.
- Standard `macos-15` runner, required and runtime-checked as ARM64. The installed `cargo-tauri` executable must itself be a native ARM64 Mach-O before desktop or iOS builds run.
- Standard `windows-2025` runner, required and runtime-checked as x64. Setup checks the Visual Studio Native Desktop workload and WebView2 Runtime before building.
- macOS setup checks Xcode and CocoaPods and installs XcodeGen `2.46.0` from its official release asset after verifying SHA-256 `4d9e34b62172d645eed6457cac13fc222569974098ef4ee9c3368bedf0196806`.

Runner/prerequisite sources checked on 2026-10-26:

- GitHub standard runner labels and architectures: <https://docs.github.com/en/enterprise-cloud@latest/actions/how-tos/write-workflows/choose-where-workflows-run/choose-the-runner-for-a-job>
- Official runner images: <https://github.com/actions/runner-images/blob/main/images/macos/macos-15-arm64-Readme.md> and <https://github.com/actions/runner-images/blob/main/images/windows/Windows2025-Readme.md>
- Tauri macOS, Windows and iOS prerequisites: <https://v2.tauri.app/start/prerequisites/>
- Tauri NSIS guidance: <https://v2.tauri.app/distribute/windows-installer/>

## Action integrity

GitHub recommends full-length immutable action SHAs: <https://docs.github.com/en/actions/reference/security/secure-use>. The workflow uses only two actions, both re-read from the owning GitHub repositories/API:

- `actions/checkout@11d5960a326750d5838078e36cf38b85af677262` (`v4.4.0`); official commit API reported `verification.verified=true` and `reason=valid`.
- `actions/upload-artifact@ea165f8d65b6e75b540449e92b4886f43607fa02` (`v4.6.2`); official commit API reported `verification.verified=true` and `reason=valid`.

Tags are comments only; execution is pinned to the exact 40-character SHAs.

## Checks and candidate artifacts

Both native jobs run format, tests and Clippy with warnings denied for the three independent Cargo workspaces (`crates/pdfmaster-core`, `app`, `src-tauri`), then build the Leptos UI for `wasm32-unknown-unknown`. The core-only smoke invokes real Rust contracts for blank creation, edit/save/reopen, and PNG rendering. It is not a headless GUI test.

Only these exact ordinary files and their exact `.sha256` files are uploaded; no `dist`, `target`, generated workspace, or wildcard path is uploaded:

- `artifacts/pdfmaster-rust-candidate-macos-arm64.zip` — exact unsigned `.app` ZIP after ARM64 Mach-O and no-symlink checks.
- `artifacts/pdfmaster-rust-candidate-windows-x64-nsis.exe` — exact unsigned NSIS installer. The built application is checked separately for x64 PE32+; the NSIS container is checked as PE32/i386 with a bounded NSIS first-header check. Its stub architecture does not establish the payload architecture or successful installation.
- `artifacts/pdfmaster-rust-candidate-ios-simulator-arm64.xcarchive.zip` — exact unsigned simulator archive after native ARM64 CLI, simulator target, archived Mach-O `LC_BUILD_VERSION` = `IOSSIMULATOR`, and plist `iPhoneSimulator`/`iphonesimulator` checks. ARM64 alone would also admit device binaries.

ZIP entries are checked for traversal, aliases/duplicates, unexpected roots and symlinks. After the 2 GiB declared uncompressed budget is checked, all member data/CRCs are read and verified. SHA manifests contain only the digest and artifact basename, avoiding private runner paths.

Container format references: [NSIS owning source](https://github.com/kichik/nsis/blob/master/Source/exehead/fileform.h), and the pinned Tauri bundler's `lzma-x86-unicode` stub selection (local registry `tauri-bundler-2.10.4/src/bundle/windows/nsis/mod.rs`). Synthetic parser fixtures are not runnable Windows acceptance evidence.

## TDD and local evidence

RED: before implementation, `PYTHONDONTWRITEBYTECODE=1 python3 -m unittest -v tools/test_candidate_ci.py` produced 6/6 expected failures because the workflow, build config, smoke script and artifact guard did not exist.

Initial GREEN on this Apple Silicon development host (before follow-up review):

- CI configuration/artifact-guard unit tests: 6/6 passed.
- `build.json` passed the `mac-win-parity` skill CI generator and produced a manual, read-only two-runner preview.
- JSON, Python, Bash and YAML syntax checks passed.
- Core-only smoke: 3/3 selected real contracts passed.
- Full local `tools/ci-check.sh`: core 77/77 tests, UI 7/7 tests, native shell 16/16 tests; all three Clippy runs passed with `-D warnings`; locked release WASM build succeeded with Trunk `0.21.14`.

No `__pycache__` is included; local Python verification uses `PYTHONDONTWRITEBYTECODE=1` or an external temporary cache.

### Review follow-up, 2026-10-26

Independent review found that the original x64 check targeted NSIS's 32-bit outer stub, iOS checks did not distinguish device vs simulator, and ZIP central-directory checks did not verify data CRCs. Regression RED: 3 newly added tests failed (6 existing passed). Fixed the three boundaries above, strengthened PE section/header bounds, and added synthetic PE32/PE32+/NSIS rejection contracts. GREEN: **10/10** configuration/artifact-guard tests and Bash syntax passed. The existing local iOS archive was independently read with `vtool` and `plutil`: `IOSSIMULATOR`, `iPhoneSimulator`, `iphonesimulator`. No Windows runner or hosted workflow execution is implied.

## Still pending

- The GitHub workflow has **not been dispatched** in this task. Therefore no hosted `macos-15` `.app`, `windows-2025` NSIS installer, or hosted iOS simulator archive is claimed.
- No Windows install/launch/save/reopen/uninstall test has run on a clean Windows machine. SmartScreen and Windows signing remain pending.
- No packaged macOS or iOS GUI was launched by CI. macOS signing/notarization, iOS device/TestFlight/App Store work, and real-device behavior remain pending.
- Visual and interaction parity remain separate human acceptance work. A successful candidate build or Rust core smoke cannot close `cross-platform/parity.json`.
- No public Git write, Release creation, artifact upload, deployment or installation occurred in this task.
