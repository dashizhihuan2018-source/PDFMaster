#!/bin/bash
# Intended only for a disposable, standard GitHub-hosted public macOS runner.
set -euo pipefail
[[ "${GITHUB_ACTIONS:-}" == true && "$(uname -m)" == arm64 ]] || { echo 'Requires disposable GitHub arm64 runner.' >&2; exit 64; }
[[ "${RUNNER_TEMP:-}" == /* && -d "$RUNNER_TEMP" ]] || exit 64
smoke_dir="$(mktemp -d "$RUNNER_TEMP/pdfmaster-release-smoke.XXXXXX")"
mount_dir="$smoke_dir/mounted"
mkdir "$mount_dir"
mounted=false
trap 'if [[ "$mounted" == true ]]; then hdiutil detach "$mount_dir" -quiet; fi' EXIT
download_url="$(plutil -extract download raw -o - docs/release.json)"
expected_sha="$(plutil -extract sha256 raw -o - docs/release.json)"
[[ "$download_url" == https://github.com/dashizhihuan2018-source/PDFMaster/releases/download/* ]] || exit 64
curl --fail --location --connect-timeout 20 --max-time 120 "$download_url" --output "$smoke_dir/release.dmg"
actual_sha="$(shasum -a 256 "$smoke_dir/release.dmg")"
[[ "${actual_sha%% *}" == "$expected_sha" ]] || { echo 'Downloaded digest mismatch.' >&2; exit 65; }
echo "Public DMG digest verified: $expected_sha"
sw_vers
uname -m
spctl --status || true
hdiutil verify "$smoke_dir/release.dmg"
hdiutil attach "$smoke_dir/release.dmg" -readonly -nobrowse -mountpoint "$mount_dir" -quiet
mounted=true
[[ -f "$mount_dir/.VolumeIcon.icns" ]] || { echo 'Missing volume icon.' >&2; exit 66; }
ditto "$mount_dir/PDF Master.app" "$smoke_dir/PDF Master.app"
hdiutil detach "$mount_dir" -quiet
mounted=false
app="$smoke_dir/PDF Master.app"
codesign --verify --deep --strict "$app"
[[ "$(plutil -extract CFBundleVersion raw -o - "$app/Contents/Info.plist")" == "$(plutil -extract build raw -o - docs/release.json)" ]] || exit 65
main_name="$(plutil -extract CFBundleExecutable raw -o - "$app/Contents/Info.plist")"
[[ "$main_name" != */* && "$(lipo -archs "$app/Contents/MacOS/$main_name")" == arm64 ]] || exit 65
env -i PATH=/usr/bin:/bin "$app/Contents/Resources/Engines/qpdf/bin/qpdf" --version
swiftc scripts/ci-release-smoke.swift -o "$smoke_dir/launch-probe"
"$smoke_dir/launch-probe" "$app" "$smoke_dir"
echo 'PASS: published bytes, bundled qpdf, synthetic PDF, Launch Services startup and normal quit.'
echo 'NOT a browser-quarantine or first-trust acceptance test. CI is not Apple notarization.'
