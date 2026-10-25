#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
project_root="$(cd "$script_dir/.." && pwd)"
cd "$project_root"

export NO_COLOR=true
rust_version="1.96.0"

cargo "+$rust_version" fmt --manifest-path crates/pdfmaster-core/Cargo.toml --all -- --check
cargo "+$rust_version" test --locked --manifest-path crates/pdfmaster-core/Cargo.toml
cargo "+$rust_version" clippy --locked --manifest-path crates/pdfmaster-core/Cargo.toml \
  --all-targets -- -D warnings

cargo "+$rust_version" fmt --manifest-path app/Cargo.toml --all -- --check
cargo "+$rust_version" test --locked --manifest-path app/Cargo.toml --lib
cargo "+$rust_version" clippy --locked --manifest-path app/Cargo.toml --all-targets -- -D warnings

cargo "+$rust_version" fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo "+$rust_version" test --locked --manifest-path src-tauri/Cargo.toml \
  --lib --test security_contract
cargo "+$rust_version" clippy --locked --manifest-path src-tauri/Cargo.toml \
  --all-targets -- -D warnings

rustup target list --toolchain "$rust_version" --installed | grep -Fx wasm32-unknown-unknown
trunk --no-color build --release --locked
