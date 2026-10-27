#!/usr/bin/env bash
set -euo pipefail

platform="${1:?usage: ci-setup.sh macos|windows}"
case "$platform" in
  macos|windows) ;;
  *) echo "unsupported CI platform: $platform" >&2; exit 2 ;;
esac

rust_version="1.96.0"
trunk_version="0.21.14"
tauri_version="2.12.4"
cargo_about_version="0.9.2"
# Must match app/Cargo.lock; avoid Trunk's deferred platform-archive download.
wasm_bindgen_version="0.2.129"
tool_root="${RUNNER_TEMP:?RUNNER_TEMP is required}/pdfmaster-rust-tools"
tool_bin="$tool_root/bin"
mkdir -p "$tool_bin"

rustup toolchain install "$rust_version" --profile minimal --component rustfmt,clippy
rustup target add --toolchain "$rust_version" wasm32-unknown-unknown
if [[ "$platform" == "macos" ]]; then
  rustup target add --toolchain "$rust_version" aarch64-apple-ios-sim aarch64-apple-ios
fi

cargo "+$rust_version" install trunk --version "$trunk_version" --locked --root "$tool_root"
cargo "+$rust_version" install tauri-cli --version "$tauri_version" --locked --root "$tool_root"
cargo "+$rust_version" install cargo-about --version "$cargo_about_version" --locked \
  --features cli --root "$tool_root"
cargo "+$rust_version" install wasm-bindgen-cli --version "$wasm_bindgen_version" --locked \
  --bin wasm-bindgen --root "$tool_root"
printf '%s\n' "$tool_bin" >> "${GITHUB_PATH:?GITHUB_PATH is required}"
export PATH="$tool_bin:$PATH"

if [[ "$platform" == "windows" ]]; then
  python_bin="python"
else
  python_bin="python3"
fi
cargo_about_bin="$(command -v cargo-about)"
printf 'CARGO_ABOUT_BIN=%s\n' "$cargo_about_bin" >> "${GITHUB_ENV:?GITHUB_ENV is required}"
printf 'PYTHON_BIN=%s\n' "$python_bin" >> "$GITHUB_ENV"
export CARGO_ABOUT_BIN="$cargo_about_bin"
export PYTHON_BIN="$python_bin"

rustc "+$rust_version" --version | grep -F "rustc $rust_version "
[[ "$(trunk --version)" == "trunk ${trunk_version}" ]]
[[ "$(cargo-tauri --version)" == "tauri-cli ${tauri_version}" ]]
[[ "$("$CARGO_ABOUT_BIN" --version)" == "cargo-about ${cargo_about_version}" ]]
[[ "$(wasm-bindgen --version)" == "wasm-bindgen ${wasm_bindgen_version}" ]]

if [[ "$platform" == "macos" ]]; then
  [[ "${RUNNER_OS:-macOS}" == "macOS" ]]
  [[ "${RUNNER_ARCH:-ARM64}" == "ARM64" ]]
  [[ "$(uname -m)" == "arm64" ]]
  file "$tool_bin/cargo-tauri" | grep -q 'Mach-O 64-bit executable arm64'
  xcodebuild -version
  pod --version

  xcodegen_version="2.46.0"
  xcodegen_sha256="4d9e34b62172d645eed6457cac13fc222569974098ef4ee9c3368bedf0196806"
  xcodegen_zip="${RUNNER_TEMP}/xcodegen-${xcodegen_version}.zip"
  xcodegen_root="$tool_root/xcodegen-${xcodegen_version}"
  curl --proto '=https' --tlsv1.2 --fail --location --retry 3 \
    --output "$xcodegen_zip" \
    "https://github.com/yonaskolb/XcodeGen/releases/download/${xcodegen_version}/xcodegen.zip"
  printf '%s  %s\n' "$xcodegen_sha256" "$xcodegen_zip" | shasum -a 256 -c -
  unzip -q "$xcodegen_zip" -d "$xcodegen_root"
  xcodegen_count="$(find "$xcodegen_root" -type f -name xcodegen -perm -111 | wc -l | tr -d ' ')"
  [[ "$xcodegen_count" == "1" ]]
  xcodegen_bin="$(find "$xcodegen_root" -type f -name xcodegen -perm -111 -print)"
  printf '%s\n' "$(dirname "$xcodegen_bin")" >> "$GITHUB_PATH"
  "$xcodegen_bin" --version | grep -F "Version: ${xcodegen_version}"
else
  [[ "${RUNNER_OS:-Windows}" == "Windows" ]]
  [[ "${RUNNER_ARCH:-X64}" == "X64" ]]
  [[ "$(uname -m)" == "x86_64" ]]
  command -v 7z >/dev/null
  command -v cygpath >/dev/null
  vswhere.exe -latest -products '*' \
    -requires Microsoft.VisualStudio.Workload.NativeDesktop \
    -property installationPath | grep -q .
  pwsh -NoProfile -NonInteractive -Command '
    $roots = @(
      "${env:ProgramFiles(x86)}\Microsoft\EdgeWebView\Application",
      "$env:ProgramFiles\Microsoft\EdgeWebView\Application"
    )
    if (-not ($roots | Where-Object { Test-Path $_ })) {
      throw "Microsoft Edge WebView2 Runtime was not found"
    }
  '
fi
