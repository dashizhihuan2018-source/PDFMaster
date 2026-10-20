#!/bin/bash
# Build a new local bundle only. Never install, launch, or clean an existing app.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd -P)"
qpdf=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --qpdf) [[ $# -ge 2 ]] || exit 64; qpdf="$2"; shift 2;;
    *) echo 'Usage: bash scripts/build-app.sh [--qpdf ABSOLUTE_EXECUTABLE]' >&2; exit 64;;
  esac
done
if [[ -n "$qpdf" ]]; then
  [[ "$qpdf" == /* && -f "$qpdf" && -x "$qpdf" ]] || { echo 'Invalid external qpdf executable.' >&2; exit 64; }
  "$qpdf" --version
fi
cd "$root"
destination="$root/dist/PDF Master.app"
[[ ! -e "$destination" && ! -L "$destination" ]] || { echo 'Output exists; choose another clean checkout or move it explicitly.' >&2; exit 73; }
command -v npm >/dev/null || { echo 'Node.js/npm are required to build the bundled canvas.' >&2; exit 69; }
(cd CanvasWeb && npm ci --ignore-scripts && npm run typecheck && npm run build)
swift build -c release --product PaperDesk
bin_dir="$(swift build -c release --show-bin-path)"
mkdir -p "$root/dist"
stage="$(mktemp -d "$root/dist/bundle.XXXXXX")"
app="$stage/PDF Master.app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources/Engines/content-edit"
install -m 755 "$bin_dir/PaperDesk" "$app/Contents/MacOS/PaperDesk"
install -m 644 "$root/Resources/Info.plist" "$app/Contents/Info.plist"
install -m 644 "$root/Resources/PDFMaster.icns" "$app/Contents/Resources/PDFMaster.icns"
install -m 644 "$root/Resources/ContentEdit/engine.py" "$app/Contents/Resources/Engines/content-edit/engine.py"
cp -R "$root/Resources/ToolIcons" "$root/Resources/ToolSkins" "$root/Resources/Licenses" "$app/Contents/Resources/"
cp -R "$root/Resources/CanvasWeb" "$app/Contents/Resources/CanvasWeb"
install -m 644 "$root/Resources/ASSETS.md" "$app/Contents/Resources/ASSETS.md"
install -m 644 "$root/DISCLAIMER.md" "$app/Contents/Resources/DISCLAIMER.md"
install -m 644 "$root/COPYRIGHT.md" "$app/Contents/Resources/COPYRIGHT.md"
codesign --force --deep --sign - "$app"
codesign --verify --deep --strict "$app"
[[ ! -e "$destination" && ! -L "$destination" ]] || { echo 'Output appeared during build; staged bundle preserved.' >&2; exit 73; }
mv "$app" "$destination"
rmdir "$stage"
echo 'Built dist/PDF Master.app; not installed or launched. External engines are not bundled.'
