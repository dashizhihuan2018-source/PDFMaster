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
review="$(mktemp -d)"
review_native="$(cygpath -w "$review")"
7z x -y "-o$review_native" "$installer" >/dev/null
mapfile -t notice_dirs < <(find "$review" -type d -name third-party-notices -print)
[[ "${#notice_dirs[@]}" == "1" ]]
python tools/verify-bundled-notices.py directory \
  target/third-party-notices "${notice_dirs[0]}"

mkdir -p artifacts
rm -f "$artifact" "$manifest"
cp "$installer" "$artifact"
python tools/ci-artifact-guard.py nsis-stub "$project_root" "$artifact"
python tools/ci-artifact-guard.py hash "$project_root" "$artifact" "$manifest"
