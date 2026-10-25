#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
project_root="$(cd "$script_dir/.." && pwd)"
cd "$project_root"

[[ "${RUNNER_ARCH:-ARM64}" == "ARM64" ]]
[[ "$(uname -m)" == "arm64" ]]
file "$(command -v cargo-tauri)" | grep -q 'Mach-O 64-bit executable arm64'
rustup target list --toolchain 1.96.0 --installed | grep -Fx aarch64-apple-ios-sim

export NO_COLOR=true
export TRUNK_BUILD_LOCKED=true
cargo tauri ios init --ci --skip-targets-install
cargo tauri ios build --debug --target aarch64-sim --ci --no-sign --archive-only -- --locked

archive="src-tauri/gen/apple/build/pdfmaster-app_iOS.xcarchive"
binary="$archive/Products/Applications/PDF Master Rust Candidate.app/PDF Master Rust Candidate"
artifact="artifacts/pdfmaster-rust-candidate-ios-simulator-arm64.xcarchive.zip"
manifest="${artifact}.sha256"
[[ -d "$archive" ]]
[[ -f "$binary" ]]
[[ -x "$binary" ]]
[[ -z "$(find "$archive" -type l -print -quit)" ]]
[[ "$(lipo -archs "$binary")" == "arm64" ]]
file "$binary" | grep -q 'Mach-O 64-bit executable arm64'
vtool -show-build "$binary" | grep -Eq '^[[:space:]]*platform IOSSIMULATOR$'
plist="$archive/Products/Applications/PDF Master Rust Candidate.app/Info.plist"
[[ "$(plutil -extract CFBundleSupportedPlatforms.0 raw -o - "$plist")" == "iPhoneSimulator" ]]
[[ "$(plutil -extract DTPlatformName raw -o - "$plist")" == "iphonesimulator" ]]

mkdir -p artifacts
rm -f "$artifact" "$manifest"
ditto -c -k --keepParent "$archive" "$artifact"
python3 tools/ci-artifact-guard.py zip \
  "$project_root" "$artifact" "pdfmaster-app_iOS.xcarchive/"
python3 tools/ci-artifact-guard.py hash \
  "$project_root" "$artifact" "$manifest"
