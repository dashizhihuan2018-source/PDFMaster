#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
project_root="$(cd "$script_dir/.." && pwd)"
cd "$project_root"

[[ "${RUNNER_ARCH:-ARM64}" == "ARM64" ]]
[[ "$(uname -m)" == "arm64" ]]
file "$(command -v cargo-tauri)" | grep -q 'Mach-O 64-bit executable arm64'

export NO_COLOR=true
export TRUNK_BUILD_LOCKED=true
cargo tauri build --ci --bundles app --no-sign -- --locked

bundle="src-tauri/target/release/bundle/macos/PDF Master Rust Candidate.app"
binary="$bundle/Contents/MacOS/pdfmaster-app"
artifact="artifacts/pdfmaster-rust-candidate-macos-arm64.zip"
manifest="${artifact}.sha256"
[[ -d "$bundle" ]]
[[ -f "$binary" ]]
[[ -x "$binary" ]]
[[ -z "$(find "$bundle" -type l -print -quit)" ]]
[[ "$(lipo -archs "$binary")" == "arm64" ]]
file "$binary" | grep -q 'Mach-O 64-bit executable arm64'

mkdir -p artifacts
rm -f "$artifact" "$manifest"
ditto -c -k --keepParent "$bundle" "$artifact"
python3 tools/ci-artifact-guard.py zip \
  "$project_root" "$artifact" "PDF Master Rust Candidate.app/"
python3 tools/ci-artifact-guard.py hash \
  "$project_root" "$artifact" "$manifest"
