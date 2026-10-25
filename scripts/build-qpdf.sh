#!/bin/bash
# Build an isolated arm64/macOS 14 qpdf payload from official unpacked sources.
# Obtain qpdf 12.4.2: https://github.com/qpdf/qpdf/releases/tag/v12.4.2
# Obtain libjpeg-turbo 3.2.0: https://github.com/libjpeg-turbo/libjpeg-turbo/releases/tag/3.2.0
# Verify vendor checksums/signatures before use. This script does not download or install.
set -euo pipefail
if [[ "${1:-}" == "--help" ]]; then
  echo 'Usage: build-qpdf.sh --qpdf-source ABS_DIR --jpeg-source ABS_DIR --output NEW_ABS_DIR'
  exit 0
fi
qpdf_source="" jpeg_source="" output=""
while [[ $# -gt 0 ]]; do
  [[ $# -ge 2 ]] || exit 64
  case "$1" in --qpdf-source) qpdf_source="$2";; --jpeg-source) jpeg_source="$2";; --output) output="$2";; *) exit 64;; esac
  shift 2
done
[[ "$output" == /* && ! -e "$output" && ! -L "$output" && "$qpdf_source" == /* && "$jpeg_source" == /* ]] || exit 64
[[ -f "$qpdf_source/CMakeLists.txt" && -f "$qpdf_source/LICENSE.txt" && -f "$jpeg_source/CMakeLists.txt" ]] || exit 66
command -v cmake >/dev/null || { echo 'CMake required.' >&2; exit 69; }
mkdir -p "$output/build" "$output/bin" "$output/lib" "$output/Licenses/qpdf" "$output/Licenses/libjpeg-turbo"
cmake -S "$jpeg_source" -B "$output/build/jpeg" -DCMAKE_BUILD_TYPE=Release -DCMAKE_OSX_ARCHITECTURES=arm64 -DCMAKE_OSX_DEPLOYMENT_TARGET=14.0 -DENABLE_SHARED=OFF -DENABLE_STATIC=ON -DWITH_TURBOJPEG=OFF -DWITH_JPEG8=ON -DWITH_SIMD=OFF
cmake --build "$output/build/jpeg" --target jpeg-static --parallel 4
mkdir -p "$output/build/jpeg-include"
cp "$jpeg_source/src/jpeglib.h" "$jpeg_source/src/jmorecfg.h" "$output/build/jpeg/jconfig.h" "$output/build/jpeg-include/"
cmake -S "$qpdf_source" -B "$output/build/qpdf" -DCMAKE_BUILD_TYPE=Release -DCMAKE_OSX_ARCHITECTURES=arm64 -DCMAKE_OSX_DEPLOYMENT_TARGET=14.0 -DPKG_CONFIG_EXECUTABLE=/usr/bin/false -DBUILD_SHARED_LIBS=ON -DBUILD_STATIC_LIBS=OFF -DREQUIRE_CRYPTO_NATIVE=ON -DUSE_IMPLICIT_CRYPTO=OFF -DSTATIC_JPEG=ON -DLIBJPEG_H_PATH="$output/build/jpeg-include" -DLIBJPEG_LIB_PATH="$output/build/jpeg/libjpeg.a" -DCMAKE_BUILD_WITH_INSTALL_RPATH=ON '-DCMAKE_INSTALL_RPATH=@loader_path/../lib' -DCMAKE_INSTALL_RPATH_USE_LINK_PATH=OFF
cmake --build "$output/build/qpdf" --target qpdf --parallel 4
install -m 755 "$output/build/qpdf/qpdf/qpdf" "$output/bin/qpdf"
install -m 755 "$output/build/qpdf/libqpdf/libqpdf.30.4.2.dylib" "$output/lib/libqpdf.30.dylib"
for name in LICENSE.txt NOTICE.md Artistic-2.0; do install -m 644 "$qpdf_source/$name" "$output/Licenses/qpdf/$name"; done
for name in LICENSE.md README.ijg; do install -m 644 "$jpeg_source/$name" "$output/Licenses/libjpeg-turbo/$name"; done
codesign --force --sign - "$output/lib/libqpdf.30.dylib"
codesign --force --sign - "$output/bin/qpdf"
env PATH=/does-not-exist "$output/bin/qpdf" --version
echo 'Portable payload ready; pass bin/qpdf, lib/libqpdf.30.dylib and original source directories to package-local-resources.sh.'
