#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
project_root="$(cd "$script_dir/.." && pwd)"
cd "$project_root"

export NO_COLOR=true
rust_version="1.96.0"
python_bin="${PYTHON_BIN:-python3}"
# Python on Windows searches System32 before Git's Bash and can launch WSL.
# Pass the exact shell already running this script, converted for CreateProcess.
if [[ "${OS:-}" == "Windows_NT" ]]; then
  export PDFMASTER_TEST_BASH="$(cygpath -w "$BASH")"
else
  export PDFMASTER_TEST_BASH="$BASH"
fi

bash tools/generate-third-party-notices.sh target/third-party-notices
PYTHONDONTWRITEBYTECODE=1 "$python_bin" -m unittest -v tools/test_third_party_notices.py

cargo "+$rust_version" fmt --manifest-path crates/pdfmaster-core/Cargo.toml --all -- --check
cargo "+$rust_version" test --locked --manifest-path crates/pdfmaster-core/Cargo.toml
cargo "+$rust_version" clippy --locked --manifest-path crates/pdfmaster-core/Cargo.toml \
  --all-targets -- -D warnings

cargo "+$rust_version" fmt --manifest-path app/Cargo.toml --all -- --check
cargo "+$rust_version" test --locked --manifest-path app/Cargo.toml --all-targets
cargo "+$rust_version" clippy --locked --manifest-path app/Cargo.toml --all-targets -- -D warnings

cargo "+$rust_version" fmt --manifest-path src-tauri/Cargo.toml --all -- --check
cargo "+$rust_version" test --locked --manifest-path src-tauri/Cargo.toml \
  --lib --test security_contract
cargo "+$rust_version" clippy --locked --manifest-path src-tauri/Cargo.toml \
  --all-targets -- -D warnings

rustup target list --toolchain "$rust_version" --installed | grep -Fx wasm32-unknown-unknown
trunk --no-color build --release --locked
