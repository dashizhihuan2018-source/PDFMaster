#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
project_root="$(cd "$script_dir/.." && pwd)"
cd "$project_root"

rust_version="1.96.0"
test_target=("+$rust_version" test --locked --manifest-path crates/pdfmaster-core/Cargo.toml \
  --test document_contract)

cargo "${test_target[@]}" blank_creates_a_parseable_pdf_with_requested_pages -- --exact
cargo "${test_target[@]}" text_edit_is_extractable_after_independent_reopen -- --exact
cargo "${test_target[@]}" render_png_has_expected_dimensions_and_nonwhite_pixels -- --exact

echo "Core-only candidate smoke passed: blank, edit/save/reopen, and render contracts."
echo "This is not packaged GUI acceptance or full parity evidence."
