#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
project_root="$(cd "$script_dir/.." && pwd)"
cd "$project_root"

[[ "${RUNNER_ARCH:-X64}" == "X64" ]]
[[ "$(uname -m)" == "x86_64" ]]

export NO_COLOR=true
export TRUNK_BUILD_LOCKED=true
cargo tauri build --ci --bundles nsis --no-sign -- --locked

installer="src-tauri/target/release/bundle/nsis/PDF Master Rust Candidate_2.0.0_x64-setup.exe"
application="src-tauri/target/release/pdfmaster-app.exe"
artifact="artifacts/pdfmaster-rust-candidate-windows-x64-nsis.exe"
manifest="${artifact}.sha256"
[[ -f "$installer" ]]
[[ ! -L "$installer" ]]
python tools/ci-artifact-guard.py pe-x64 "$project_root" "$application"

mkdir -p artifacts
rm -f "$artifact" "$manifest"
cp "$installer" "$artifact"
python tools/ci-artifact-guard.py nsis-stub "$project_root" "$artifact"
python tools/ci-artifact-guard.py hash "$project_root" "$artifact" "$manifest"
