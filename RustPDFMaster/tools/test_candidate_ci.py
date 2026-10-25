#!/usr/bin/env python3
import json
import os
from pathlib import Path
import struct
import subprocess
import sys
import tempfile
import unittest
import zipfile


PROJECT = Path(__file__).resolve().parents[1]
REPOSITORY = PROJECT.parent
WORKFLOW = REPOSITORY / ".github/workflows/pdfmaster-rust-candidate.yml"
BUILD_CONFIG = PROJECT / "cross-platform/build.json"
TOOLS = PROJECT / "tools"
CHECKOUT_SHA = "11d5960a326750d5838078e36cf38b85af677262"
UPLOAD_SHA = "ea165f8d65b6e75b540449e92b4886f43607fa02"


def synthetic_pe(machine=0x8664):
    """Parser fixture only; not a runnable Windows application/installer."""
    optional_size = 240 if machine == 0x8664 else 224
    data = bytearray(1024)
    data[:2] = b"MZ"
    struct.pack_into("<I", data, 0x3C, 0x80)
    data[0x80:0x84] = b"PE\0\0"
    struct.pack_into("<HH", data, 0x84, machine, 1)
    struct.pack_into("<HH", data, 0x94, optional_size, 2)
    struct.pack_into("<H", data, 0x98, 0x20B if machine == 0x8664 else 0x10B)
    struct.pack_into("<II", data, 0x98 + optional_size + 16, 512, 512)
    return data


class CandidateCiContract(unittest.TestCase):
    def read_required(self, path: Path) -> str:
        self.assertTrue(path.is_file(), f"missing required file: {path}")
        return path.read_text(encoding="utf-8")

    def guard(self, root, command, *arguments):
        return subprocess.run([sys.executable, str(TOOLS / "ci-artifact-guard.py"), command, str(root), *arguments], capture_output=True, text=True)

    def test_windows_application_and_nsis_stub_have_separate_contracts(self):
        script = self.read_required(TOOLS / "ci-build-windows.sh")
        self.assertIn('application="src-tauri/target/release/pdfmaster-app.exe"', script)
        self.assertIn('pe-x64 "$project_root" "$application"', script)
        self.assertIn('nsis-stub "$project_root" "$artifact"', script)

    def test_ios_archive_requires_simulator_platform_not_only_arm64(self):
        script = self.read_required(TOOLS / "ci-build-ios.sh")
        self.assertIn('vtool -show-build', script)
        self.assertIn('IOSSIMULATOR', script)
        self.assertIn('CFBundleSupportedPlatforms.0', script)
        self.assertIn('iPhoneSimulator', script)

    def test_zip_corrupt_member_data_is_rejected_before_hash(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            path = root / "corrupt.zip"
            with zipfile.ZipFile(path, "w", compression=zipfile.ZIP_STORED) as archive:
                archive.writestr("Candidate.app/payload", b"ORIGINAL")
            data = bytearray(path.read_bytes())
            with zipfile.ZipFile(path) as archive:
                info = archive.infolist()[0]
                start = info.header_offset + 30 + len(info.filename.encode()) + len(info.extra)
            data[start] ^= 1
            path.write_bytes(data)
            self.assertNotEqual(self.guard(root, "zip", "corrupt.zip", "Candidate.app/").returncode, 0)

    def test_pe_and_nsis_synthetic_parser_contracts(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            path = root / "fixture.exe"
            path.write_bytes(synthetic_pe())
            self.assertEqual(self.guard(root, "pe-x64", path.name).returncode, 0)
            self.assertNotEqual(self.guard(root, "nsis-stub", path.name).returncode, 0)
            stub = synthetic_pe(0x14C)
            path.write_bytes(stub)
            self.assertNotEqual(self.guard(root, "pe-x64", path.name).returncode, 0)
            self.assertNotEqual(self.guard(root, "nsis-stub", path.name).returncode, 0)
            stub += struct.pack("<7I", 4, 0xDEADBEEF, 0x6C6C754E, 0x74666F73, 0x74736E49, 4, 32) + b"data"
            path.write_bytes(stub)
            self.assertEqual(self.guard(root, "nsis-stub", path.name).returncode, 0)
            path.write_bytes(stub[:-1])
            self.assertNotEqual(self.guard(root, "nsis-stub", path.name).returncode, 0)
            path.write_bytes(synthetic_pe()[:300])
            self.assertNotEqual(self.guard(root, "pe-x64", path.name).returncode, 0)

    def test_workflow_is_manual_read_only_bounded_and_sha_pinned(self):
        workflow = self.read_required(WORKFLOW)
        self.assertIn("workflow_dispatch:", workflow)
        self.assertNotIn("\n  push:", workflow)
        self.assertNotIn("\n  pull_request:", workflow)
        self.assertNotIn("\n  schedule:", workflow)
        self.assertIn("permissions:\n  contents: read", workflow)
        self.assertIn("concurrency:", workflow)
        self.assertGreaterEqual(workflow.count("timeout-minutes:"), 2)
        self.assertNotIn("secrets.", workflow)
        self.assertIn(f"actions/checkout@{CHECKOUT_SHA}", workflow)
        self.assertGreaterEqual(
            workflow.count(f"actions/upload-artifact@{UPLOAD_SHA}"), 3
        )
        for line in workflow.splitlines():
            if "uses:" in line:
                reference = line.rsplit("@", 1)[-1].split()[0]
                self.assertRegex(reference, r"^[0-9a-f]{40}$")

    def test_workflow_uses_standard_native_runners_and_exact_artifacts(self):
        workflow = self.read_required(WORKFLOW)
        self.assertIn("runs-on: macos-15", workflow)
        self.assertIn("runs-on: windows-2025", workflow)
        expected = [
            "artifacts/pdfmaster-rust-candidate-macos-arm64.zip",
            "artifacts/pdfmaster-rust-candidate-macos-arm64.zip.sha256",
            "artifacts/pdfmaster-rust-candidate-ios-simulator-arm64.xcarchive.zip",
            "artifacts/pdfmaster-rust-candidate-ios-simulator-arm64.xcarchive.zip.sha256",
            "artifacts/pdfmaster-rust-candidate-windows-x64-nsis.exe",
            "artifacts/pdfmaster-rust-candidate-windows-x64-nsis.exe.sha256",
        ]
        for path in expected:
            self.assertIn(path, workflow)
        self.assertNotIn("artifacts/**", workflow)
        self.assertNotIn("target/**", workflow)

    def test_build_config_has_real_two_platform_commands(self):
        config = json.loads(self.read_required(BUILD_CONFIG))
        self.assertEqual(config["schema_version"], 1)
        self.assertEqual(config["project"], "pdfmaster-rust-candidate")
        self.assertEqual(set(config["jobs"]), {"macos", "windows"})
        self.assertEqual(config["jobs"]["macos"]["runner"], "macos-15")
        self.assertEqual(config["jobs"]["windows"]["runner"], "windows-2025")
        for platform in ("macos", "windows"):
            job = config["jobs"][platform]
            for phase in ("setup", "test", "build", "smoke", "artifact"):
                self.assertTrue(job[phase].strip())
            self.assertNotIn("exit 1", "\n".join(job.values()))

    def test_scripts_pin_tools_check_all_workspaces_and_build_wasm(self):
        scripts = "\n".join(
            path.read_text(encoding="utf-8")
            for path in sorted(TOOLS.glob("ci-*.sh"))
        )
        for version in ("1.96.0", "0.21.14", "2.12.4"):
            self.assertIn(version, scripts)
        for workspace in (
            "crates/pdfmaster-core",
            "app/Cargo.toml",
            "src-tauri/Cargo.toml",
        ):
            self.assertIn(workspace, scripts)
        self.assertIn("wasm32-unknown-unknown", scripts)
        self.assertIn("trunk --no-color build --release", scripts)
        self.assertIn("--locked", scripts)
        self.assertIn("clippy", scripts)
        self.assertIn("fmt", scripts)
        self.assertIn("cargo tauri build --ci --bundles app --no-sign", scripts)
        self.assertIn("cargo tauri build --ci --bundles nsis --no-sign", scripts)
        self.assertIn(
            "cargo tauri ios build --debug --target aarch64-sim --ci --no-sign --archive-only",
            scripts,
        )
        self.assertIn("cargo-tauri", scripts)
        self.assertIn("arm64", scripts)
        self.assertNotIn("--sequesterRsrc", scripts)

    def test_smoke_is_a_real_core_lifecycle_subset_not_gui_parity(self):
        smoke = self.read_required(TOOLS / "ci-core-smoke.sh")
        for contract in (
            "blank_creates_a_parseable_pdf_with_requested_pages",
            "text_edit_is_extractable_after_independent_reopen",
            "render_png_has_expected_dimensions_and_nonwhite_pixels",
        ):
            self.assertIn(contract, smoke)
        self.assertIn("not packaged GUI acceptance or full parity evidence", smoke)

    def test_artifact_guard_hash_zip_and_pe_contracts(self):
        guard = TOOLS / "ci-artifact-guard.py"
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            safe_zip = root / "candidate.zip"
            with zipfile.ZipFile(safe_zip, "w") as archive:
                archive.writestr("Candidate.app/Contents/Info.plist", "safe")
            result = subprocess.run(
                [
                    sys.executable,
                    str(guard),
                    "zip",
                    str(root),
                    "candidate.zip",
                    "Candidate.app/",
                ],
                capture_output=True,
                text=True,
            )
            self.assertEqual(result.returncode, 0, result.stderr)

            digest = subprocess.run(
                [
                    sys.executable,
                    str(guard),
                    "hash",
                    str(root),
                    "candidate.zip",
                    "candidate.zip.sha256",
                ],
                capture_output=True,
                text=True,
            )
            self.assertEqual(digest.returncode, 0, digest.stderr)
            manifest = (root / "candidate.zip.sha256").read_text(encoding="ascii")
            self.assertRegex(manifest, r"^[0-9a-f]{64}  candidate\.zip\n$")

            unsafe_zip = root / "unsafe.zip"
            with zipfile.ZipFile(unsafe_zip, "w") as archive:
                archive.writestr("../escape", "unsafe")
            result = subprocess.run(
                [
                    sys.executable,
                    str(guard),
                    "zip",
                    str(root),
                    "unsafe.zip",
                    "Candidate.app/",
                ],
                capture_output=True,
                text=True,
            )
            self.assertNotEqual(result.returncode, 0)

            pe = synthetic_pe()
            (root / "candidate.exe").write_bytes(pe)
            result = subprocess.run(
                [
                    sys.executable,
                    str(guard),
                    "pe-x64",
                    str(root),
                    "candidate.exe",
                ],
                capture_output=True,
                text=True,
            )
            self.assertEqual(result.returncode, 0, result.stderr)

            if hasattr(os, "symlink"):
                try:
                    os.symlink(safe_zip, root / "linked.zip")
                except OSError:
                    pass
                else:
                    result = subprocess.run(
                        [
                            sys.executable,
                            str(guard),
                            "hash",
                            str(root),
                            "linked.zip",
                            "linked.zip.sha256",
                        ],
                        capture_output=True,
                        text=True,
                    )
                    self.assertNotEqual(result.returncode, 0)


if __name__ == "__main__":
    unittest.main()
