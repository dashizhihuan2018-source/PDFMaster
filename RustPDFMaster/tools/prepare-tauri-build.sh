#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
project_root="$(cd "$script_dir/.." && pwd)"
cd "$project_root"

python_bin="${PYTHON_BIN:-python3}"
notice_source="target/third-party-notices"
bash tools/generate-third-party-notices.sh "$notice_source"

# cargo-mobile's generated Xcode project bundles this folder reference. Tauri's
# desktop resource map remains authoritative; this explicit copy is required
# because bundle.resources is not automatically added to the Xcode resources phase.
ios_assets="src-tauri/gen/apple/assets"
ios_notices="src-tauri/gen/apple/assets/third-party-notices"
if [[ -d "$ios_assets" ]]; then
  if [[ -L "$ios_assets" || -L "$ios_notices" ]]; then
    echo "iOS notice asset path must not be a symbolic link" >&2
    exit 2
  fi
  mkdir -p "$ios_notices"
  for source in "$notice_source"/*.html; do
    install -m 0644 "$source" "$ios_notices/$(basename "$source")"
  done
  PYTHONDONTWRITEBYTECODE=1 "$python_bin" tools/verify-bundled-notices.py \
    directory "$notice_source" "$ios_notices"
fi

trunk --no-color build --release --locked
