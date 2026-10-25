#!/bin/bash
# Build only; never install, launch, or clean an existing app.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd -P)"
if [[ "${1:-}" == "--help" ]]; then
  echo 'Usage: bash scripts/build-app.sh --qpdf ABS_BIN --qpdf-lib ABS_DYLIB --qpdf-source ABS_SOURCE --jpeg-source ABS_SOURCE'
  echo 'Build the portable qpdf payload with scripts/build-qpdf.sh first.'
  exit 0
fi
resource_options=("$@")
qpdf="" qpdf_lib="" qpdf_source="" jpeg_source=""
while [[ $# -gt 0 ]]; do
  [[ $# -ge 2 ]] || exit 64
  case "$1" in --qpdf) qpdf="$2";; --qpdf-lib) qpdf_lib="$2";; --qpdf-source) qpdf_source="$2";; --jpeg-source) jpeg_source="$2";; *) exit 64;; esac
  shift 2
done
for file in "$qpdf" "$qpdf_lib" "$qpdf_source/LICENSE.txt" "$jpeg_source/LICENSE.md"; do
  [[ "$file" == /* && -f "$file" ]] || { echo 'Required absolute qpdf binary/library and license source inputs missing.' >&2; exit 64; }
done
[[ -x "$qpdf" ]] || exit 64
"$qpdf" --version
cd "$root"
destination="$root/dist/PDF Master.app"
[[ ! -e "$destination" && ! -L "$destination" ]] || { echo 'Output exists; use clean staging or move it explicitly.' >&2; exit 73; }
command -v npm >/dev/null || { echo 'Node.js/npm required to build canvas.' >&2; exit 69; }
(cd CanvasWeb && npm ci --ignore-scripts && npm run typecheck && npm run build)
swift build -c release --product PaperDesk
bin_dir="$(swift build -c release --show-bin-path)"
mkdir -p "$root/dist"
stage="$(mktemp -d "$root/dist/bundle.XXXXXX")"
app="$stage/PDF Master.app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
install -m 755 "$bin_dir/PaperDesk" "$app/Contents/MacOS/PaperDesk"
install -m 644 "$root/Resources/Info.plist" "$app/Contents/Info.plist"
install -m 644 "$root/Resources/PDFMaster.icns" "$app/Contents/Resources/PDFMaster.icns"
bash "$root/scripts/package-local-resources.sh" "$app" "${resource_options[@]}"
for document in ASSETS.md; do install -m 644 "$root/Resources/$document" "$app/Contents/Resources/$document"; done
for document in DISCLAIMER.md COPYRIGHT.md; do install -m 644 "$root/$document" "$app/Contents/Resources/$document"; done
codesign --force --deep --sign - "$app"
codesign --verify --deep --strict "$app"
[[ ! -e "$destination" && ! -L "$destination" ]] || { echo 'Output appeared; staged bundle preserved.' >&2; exit 73; }
mv "$app" "$destination"
rmdir "$stage"
echo 'Built dist/PDF Master.app with native editing and bundled qpdf; not installed or launched. Ad-hoc signed, not notarized.'
