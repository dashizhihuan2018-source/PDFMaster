#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
project_root="$(cd "$script_dir/.." && pwd)"
cd "$project_root"
python_bin="${PYTHON_BIN:-python3}"

cargo_about_version="0.9.2"
cargo_about="${CARGO_ABOUT_BIN:-cargo-about}"
if [[ "$cargo_about" != */* ]]; then
  cargo_about="$(command -v "$cargo_about" || true)"
fi
if [[ -z "$cargo_about" || ! -x "$cargo_about" ]]; then
  echo "cargo-about ${cargo_about_version} is required; set CARGO_ABOUT_BIN to its executable" >&2
  exit 2
fi
if [[ "$("$cargo_about" --version)" != "cargo-about ${cargo_about_version}" ]]; then
  echo "cargo-about ${cargo_about_version} is required" >&2
  exit 2
fi

output_dir="${1:-target/third-party-notices}"

reject_symlink_components() {
  local candidate="$1"
  local probe parent
  if [[ "$candidate" != /* ]]; then
    candidate="$project_root/$candidate"
  fi
  probe="$candidate"
  while true; do
    if [[ -L "$probe" ]]; then
      echo "notice output path contains a symbolic link component" >&2
      return 2
    fi
    [[ "$probe" == "/" ]] && break
    parent="$(dirname "$probe")"
    [[ "$parent" == "$probe" ]] && break
    probe="$parent"
  done
}

reject_symlink_components "$output_dir"

stage_root="$project_root/target/notice-staging"
reject_symlink_components "$stage_root"
mkdir -p "$stage_root"
reject_symlink_components "$stage_root"
stage="$(mktemp -d "$stage_root/run.XXXXXX")"
trap 'rm -rf -- "$stage"' EXIT
template="$project_root/templates/third-party-notices.hbs"
config="$project_root/about.toml"
entries=(
  "src-tauri/Cargo.toml|aarch64-apple-darwin|macos-aarch64.html"
  "src-tauri/Cargo.toml|x86_64-pc-windows-msvc|windows-x86_64.html"
  "src-tauri/Cargo.toml|aarch64-apple-ios-sim|ios-simulator-aarch64.html"
  "src-tauri/Cargo.toml|aarch64-apple-ios|ios-device-aarch64.html"
  "app/Cargo.toml|wasm32-unknown-unknown|ui-wasm32.html"
)

for entry in "${entries[@]}"; do
  IFS='|' read -r manifest target output <<< "$entry"
  "$cargo_about" generate "$template" \
    --manifest-path "$project_root/$manifest" \
    --config "$config" \
    --locked \
    --target "$target" \
    --fail \
    --output-file "$stage/$output"
done

PYTHONDONTWRITEBYTECODE=1 "$python_bin" "$script_dir/validate-third-party-notices.py" "$stage"
mkdir -p "$output_dir"
reject_symlink_components "$output_dir"
for entry in "${entries[@]}"; do
  output="${entry##*|}"
  reject_symlink_components "$output_dir/$output"
  if [[ -L "$output_dir/$output" ]]; then
    echo "$output: existing notice output must not be a symbolic link" >&2
    exit 2
  fi
  install -m 0644 "$stage/$output" "$output_dir/$output"
done
PYTHONDONTWRITEBYTECODE=1 "$python_bin" "$script_dir/validate-third-party-notices.py" "$output_dir"
echo "Generated five validated target-specific third-party notices."
