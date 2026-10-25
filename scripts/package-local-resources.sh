#!/bin/bash
# Resource-only staging; no installation, launch, paid service, or network access.
set -euo pipefail
repo_dir="$(cd "$(dirname "$0")/.." && pwd -P)"
stage="${1:?Usage: package-local-resources.sh ABSOLUTE_STAGE --qpdf BIN --qpdf-lib DYLIB --qpdf-source SOURCE --jpeg-source SOURCE [--canvas DIR]}"
shift
qpdf_bin="" qpdf_lib="" qpdf_source="" jpeg_source="" canvas="$repo_dir/Resources/CanvasWeb"
while [[ $# -gt 0 ]]; do
  [[ $# -ge 2 ]] || exit 64
  case "$1" in
    --qpdf) qpdf_bin="$2";;
    --qpdf-lib) qpdf_lib="$2";;
    --qpdf-source) qpdf_source="$2";;
    --jpeg-source) jpeg_source="$2";;
    --canvas) canvas="$2";;
    *) echo "Unknown resource option." >&2; exit 64;;
  esac
  shift 2
done
[[ "$stage" == /* && -d "$stage/Contents/Resources" && ! -L "$stage" ]] || exit 64
for input in "$qpdf_bin" "$qpdf_lib" "$qpdf_source/LICENSE.txt" "$qpdf_source/NOTICE.md" "$qpdf_source/Artistic-2.0" "$jpeg_source/LICENSE.md" "$jpeg_source/README.ijg" "$canvas/index.html"; do
  [[ "$input" == /* && -f "$input" ]] || { echo "Missing required absolute resource input." >&2; exit 66; }
done
[[ -x "$qpdf_bin" ]] || exit 66
"$qpdf_bin" --version | /usr/bin/grep -F 'qpdf version 12.4.2' >/dev/null
resources="$stage/Contents/Resources"
[[ ! -e "$resources/Engines/qpdf" && ! -e "$resources/Engines/content-edit" ]] || { echo "Use clean resource staging." >&2; exit 73; }
mkdir -p "$resources/Engines/qpdf/bin" "$resources/Engines/qpdf/lib" "$resources/Licenses/qpdf" "$resources/Licenses/libjpeg-turbo" "$resources/Licenses/InfiniteCanvas"
ditto "$canvas" "$resources/CanvasWeb"
ditto "$repo_dir/Resources/ToolIcons" "$resources/ToolIcons"
ditto "$repo_dir/Resources/ToolSkins" "$resources/ToolSkins"
ditto "$repo_dir/Resources/Licenses" "$resources/Licenses"
install -m 644 "$repo_dir/CanvasWeb/LICENSE" "$resources/Licenses/InfiniteCanvas/LICENSE"
install -m 755 "$qpdf_bin" "$resources/Engines/qpdf/bin/qpdf"
install -m 755 "$qpdf_lib" "$resources/Engines/qpdf/lib/libqpdf.30.dylib"
for name in LICENSE.txt NOTICE.md Artistic-2.0; do install -m 644 "$qpdf_source/$name" "$resources/Licenses/qpdf/$name"; done
for name in LICENSE.md README.ijg; do install -m 644 "$jpeg_source/$name" "$resources/Licenses/libjpeg-turbo/$name"; done
for binary in "$resources/Engines/qpdf/bin/qpdf" "$resources/Engines/qpdf/lib/libqpdf.30.dylib"; do
  while IFS= read -r dependency; do
    case "$dependency" in @rpath/*|/usr/lib/*|/System/*) ;; *) echo "Nonportable qpdf dependency; use scripts/build-qpdf.sh." >&2; exit 65;; esac
  done < <(/usr/bin/otool -L "$binary" | /usr/bin/tail -n +2 | /usr/bin/awk '{print $1}')
  codesign --force --sign - "$binary"
  codesign --verify --strict "$binary"
done
env PATH=/does-not-exist "$resources/Engines/qpdf/bin/qpdf" --version
[[ -z "$(find "$resources" -type f \( -name '*.py' -o -name '*.pyc' -o -name '*.pyo' \) -print -quit)" ]] || { echo "Python research payload is prohibited." >&2; exit 65; }
echo 'Native resources staged. Ad-hoc signatures are not Apple notarization.'
